use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
};

use crate::{
    Pid, Process, ProcessState, SchedulingDecision, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};

pub trait ProcessContainer {
    fn get_next_process(&mut self, timeslice: NonZeroUsize) -> Option<CurrentProcessMeta>;
    fn push_back(&mut self, process: ProcessMeta);
    fn append(&mut self, iter: Box<dyn Iterator<Item = ProcessMeta>>) {
        iter.into_iter().for_each(|x| self.push_back(x))
    }

    fn get_iterator<'a>(&'a self) -> Box<dyn ExactSizeIterator<Item = &'a ProcessMeta> + 'a>;
    fn get_mut_iterator<'a>(
        &'a mut self,
    ) -> Box<dyn ExactSizeIterator<Item = &'a mut ProcessMeta> + 'a>;
}

#[derive(Debug)]
pub struct ProcessMeta {
    pub(super) pid: Pid,
    pub(super) state: ProcessState,
    pub(super) last_update: usize,
    pub(super) priority: i8,
    max_priority: i8,
    /// Information about the process' run time
    /// Tota, Syscall, Execution
    timings: (usize, usize, usize),
}

impl ProcessMeta {
    fn new(pid: Pid, priority: i8, creation_time: usize) -> Self {
        ProcessMeta {
            pid,
            priority,
            max_priority: priority,
            last_update: creation_time,
            state: ProcessState::Ready,
            timings: (0, 0, 0),
        }
    }

    pub(super) fn increase_priority(&mut self) {
        self.priority += 1;
        self.priority = self.priority.min(self.max_priority);
    }

    pub(super) fn decrease_priority(&mut self) {
        self.priority -= 1;
        self.priority = self.priority.max(0);
    }
}

impl Process for ProcessMeta {
    fn pid(&self) -> Pid {
        self.pid
    }

    fn state(&self) -> ProcessState {
        self.state
    }

    fn timings(&self) -> (usize, usize, usize) {
        self.timings
    }

    fn priority(&self) -> i8 {
        self.priority
    }

    fn extra(&self) -> String {
        String::new()
    }
}

#[derive(Debug)]
/// Information about the process that is currently using the CPU.
pub struct CurrentProcessMeta {
    /// The current process.
    pub process: ProcessMeta,
    /// The cycles that the process has executed on this run.
    pub execution_cycles: usize,
    /// The cycles that the process has spent in syscalls on this run.
    pub syscall_cycles: usize,
    /// The time this process has left before it is preempted.
    pub remaining_timeslice: usize,
}
pub struct ProcessManager {
    /// The maximum timeslice that is given to each process.
    pub timeslice: NonZeroUsize,
    /// The minimum timeslice that a process must have left
    /// after a syscall in order to remain on the CPU.
    pub minimum_remaining_timeslice: usize,
    /// Wether the scheduler will panic on the next query.
    pub will_panic: bool,
    /// The maximum pid that has been assigned yet.
    pub max_pid: usize,
    pub processes: Box<dyn ProcessContainer + Send>,
    pub sleeping_processes: VecDeque<(ProcessMeta, usize)>,
    pub waiting_processes: HashMap<usize, VecDeque<ProcessMeta>>,
    /// The process that is currently running (or None if there isn't one).
    pub current_process: Option<CurrentProcessMeta>,
    /// The number of clock cycles that have passed since the scheduler was started.
    pub clock: usize,
}

impl ProcessManager {
    pub fn new(
        container: Box<impl ProcessContainer + Send + 'static>,
        timeslice: NonZeroUsize,
        minimum_remaining_timeslice: usize,
    ) -> Self {
        Self {
            minimum_remaining_timeslice,
            processes: container,
            sleeping_processes: VecDeque::new(),
            waiting_processes: HashMap::new(),
            current_process: None,
            will_panic: false,
            max_pid: 0,
            timeslice,
            clock: 0,
        }
    }

    pub fn next(&mut self) -> SchedulingDecision {
        self.update_timings();
        self.wake_processes();

        if self.will_panic {
            return SchedulingDecision::Panic;
        }

        if let Some(current) = &self.current_process {
            if current.process.state() == ProcessState::Running {
                return SchedulingDecision::Run {
                    pid: current.process.pid(),
                    timeslice: NonZeroUsize::new(current.remaining_timeslice).unwrap(),
                };
            }
        }

        if let Some(current) = self.current_process.take() {
            self.processes.push_back(current.process);
        }

        self.current_process = self.processes.get_next_process(self.timeslice);

        if let Some(current) = &self.current_process {
            return SchedulingDecision::Run {
                pid: current.process.pid(),
                timeslice: NonZeroUsize::new(current.remaining_timeslice).unwrap(),
            };
        }

        // There aren't any ready processes, wait for the first to wake up.
        if let Some(first_wake) = self
            .sleeping_processes
            .iter()
            .map(|(_, wake_time)| wake_time)
            .min()
        {
            let wait_interval = first_wake - self.clock;
            self.clock = *first_wake;

            return SchedulingDecision::Sleep(NonZeroUsize::new(wait_interval).unwrap());
        } else {
            // There are no processes to be awaken. If there still
            // are processes waiting, signal a deadlock.
            if
            /*self.0.processes.len() != 0 ||*/
            self.waiting_processes.len() != 0 {
                return SchedulingDecision::Deadlock;
            } else {
                return SchedulingDecision::Done;
            }
        }
    }

    pub fn get_processes(&self) -> Vec<&ProcessMeta> {
        self.processes
            .get_iterator()
            .chain(self.sleeping_processes.iter().map(|(x, _)| x))
            .chain(self.waiting_processes.values().flatten())
            .collect()
    }

    /// Wake up all processes that finished waiting.
    pub fn wake_processes(&mut self) {
        let sleeping = self.sleeping_processes.drain(..);
        let (awaken, sleeping) = sleeping.partition(|(_, wake_time)| *wake_time <= self.clock);
        self.sleeping_processes = sleeping;

        let awaken = awaken.into_iter().map(|(process, _)| ProcessMeta {
            state: ProcessState::Ready,
            ..process
        });

        self.processes.append(Box::new(awaken));
    }

    pub fn update_timings(&mut self) {
        let processes = self.processes.get_mut_iterator();

        let sleeping_processes = self.sleeping_processes.iter_mut();
        let sleeping_processes = sleeping_processes.map(|(process, _)| process);

        let waiting_processes = self.waiting_processes.values_mut().flatten();

        for i in processes.chain(sleeping_processes).chain(waiting_processes) {
            let elapsed_time = self.clock - i.last_update;
            i.last_update = self.clock;

            i.timings.0 += elapsed_time;
            if i.state == ProcessState::Running {
                i.timings.2 += elapsed_time;
            }
        }
    }

    pub fn handle_stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall { syscall, remaining } => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice - remaining;
                    current.process.timings.0 += 1;
                    current.process.timings.1 += 1;
                    current.syscall_cycles += 1;
                }

                let syscall_result = match syscall {
                    Fork(priority) => {
                        self.max_pid += 1;
                        let new_process =
                            ProcessMeta::new(Pid::new(self.max_pid), priority, self.clock);
                        let new_pid = new_process.pid();
                        self.processes.push_back(new_process);

                        SyscallResult::Pid(new_pid)
                    }
                    Exit => {
                        if let Some(current) = self.current_process.take() {
                            // Killing `init` while other processes are
                            // running will result in a panic.
                            if current.process.pid() == 1
                                && (self.processes.get_iterator().len() != 0
                                    || self.sleeping_processes.len() != 0
                                    || self.waiting_processes.len() != 0)
                            {
                                self.will_panic = true;
                            }

                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Signal(event) => {
                        if let Some(waiting_processes) = self.waiting_processes.remove(&event) {
                            let waiting_processes =
                                waiting_processes.into_iter().map(|mut process| {
                                    process.state = ProcessState::Ready;
                                    process
                                });

                            self.processes.append(Box::new(waiting_processes));
                        }
                        SyscallResult::Success
                    }
                    Wait(event) => {
                        if let Some(mut current) = self.current_process.take() {
                            current.process.state = ProcessState::Waiting { event: Some(event) };

                            let execution_time = self.clock - current.process.last_update;
                            if execution_time > current.syscall_cycles {
                                current.execution_cycles += execution_time - current.syscall_cycles;
                            }
                            current.process.timings.0 += current.execution_cycles;
                            current.process.timings.2 += current.execution_cycles;
                            current.process.last_update = self.clock;
                            self.waiting_processes
                                .entry(event)
                                .or_insert_with(VecDeque::new)
                                .push_back(current.process);
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Sleep(time) => {
                        if let Some(CurrentProcessMeta {
                            mut process,
                            mut execution_cycles,
                            syscall_cycles,
                            ..
                        }) = self.current_process.take()
                        {
                            process.state = ProcessState::Waiting { event: None };
                            let execution_time = self.clock - process.last_update;
                            if execution_time > syscall_cycles {
                                execution_cycles += execution_time - syscall_cycles;
                            }
                            process.timings.0 += execution_cycles;
                            process.timings.2 += execution_cycles;
                            process.last_update = self.clock;

                            self.sleeping_processes
                                .push_back((process, self.clock + time));
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                };

                // Update the timer.
                if let Some(CurrentProcessMeta {
                    execution_cycles,
                    syscall_cycles,
                    remaining_timeslice,
                    process,
                    ..
                }) = self.current_process.as_mut()
                {
                    if process.state == ProcessState::Running {
                        let execution_time = *remaining_timeslice - remaining;
                        if execution_time > *syscall_cycles {
                            *execution_cycles += execution_time - *syscall_cycles;
                        }
                        *remaining_timeslice = remaining;
                        process.last_update = self.clock;
                    }
                }
                self.update_timings();

                if let Some(current) = self.current_process.as_mut() {
                    if current.remaining_timeslice < self.minimum_remaining_timeslice {
                        current.process.state = ProcessState::Ready;
                        current.process.timings.0 += current.execution_cycles;
                        current.process.timings.2 += current.execution_cycles;
                    }
                }
                syscall_result
            }
            StopReason::Expired => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice;
                    current.process.state = ProcessState::Ready;
                    current.process.timings.0 += current.remaining_timeslice;
                    current.process.timings.2 += current.remaining_timeslice;
                    current.process.last_update = self.clock;

                    self.update_timings();

                    SyscallResult::Success
                } else {
                    SyscallResult::NoRunningProcess
                }
            }
        }
    }
}
