use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
};

use crate::{
    Pid, Process, ProcessState, SchedulingDecision, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};

#[derive(Debug)]
pub struct ProcessMeta {
    pub(super) pid: Pid,
    pub(super) state: ProcessState,
    pub(super) last_update: usize,
    pub(super) priority: i8,
    /// Information about the process' run time
    /// Tota, Syscall, Execution
    timings: (usize, usize, usize),
}

pub trait ProcessInformation: Process + Sized {
    fn set_state(&mut self, state: ProcessState);
    fn last_update(&self) -> usize;
    fn set_last_update(&mut self, last_update: usize);
    fn add_total_time(&mut self, time: usize);
    fn add_execution_time(&mut self, time: usize);
    fn add_syscall(&mut self);
    fn as_process(&self) -> &dyn Process;
    fn increase_priority(&mut self) {}
    fn decrease_priority(&mut self) {}
    fn vruntime(&self) -> usize {
        0
    }

    fn alloc(scheduler: &ProcessManager<Self>, priority: i8) -> Self;
    fn get_next_process(
        processes: &mut VecDeque<Self>,
        current_process: &mut Option<CurrentProcessMeta<Self>>,
        timeslice_factor: &mut usize,
        timeslice: NonZeroUsize,
    );
}

impl ProcessInformation for ProcessMeta {
    fn set_state(&mut self, state: ProcessState) {
        self.state = state;
    }

    fn last_update(&self) -> usize {
        self.last_update
    }

    fn set_last_update(&mut self, last_update: usize) {
        self.last_update = last_update;
    }

    fn add_total_time(&mut self, time: usize) {
        self.timings.0 += time;
    }

    fn add_execution_time(&mut self, time: usize) {
        self.timings.0 += time;
        self.timings.2 += time;
    }

    fn add_syscall(&mut self) {
        self.timings.0 += 1;
        self.timings.1 += 1;
    }

    fn as_process(&self) -> &dyn Process {
        self
    }
    fn alloc(scheduler: &ProcessManager<Self>, priority: i8) -> Self {
        Self::new(Pid::new(scheduler.max_pid), priority, scheduler.clock)
    }

    fn get_next_process(
        processes: &mut VecDeque<Self>,
        current_process: &mut Option<CurrentProcessMeta<Self>>,
        _timeslice_factor: &mut usize,
        timeslice: NonZeroUsize,
    ) {
        if let Some(current) = current_process {
            if current.process.state() == ProcessState::Running {
                return;
            }
        }

        if let Some(current) = current_process.take() {
            processes.push_back(current.process);
        }
        let mut waiting_processes = VecDeque::new();

        while let Some(mut process) = processes.pop_front() {
            if process.state() == ProcessState::Ready {
                process.set_state(ProcessState::Running);
                processes.extend(waiting_processes);

                *current_process = Some(CurrentProcessMeta {
                    process,
                    remaining_timeslice: timeslice.get(),
                    execution_cycles: 0,
                    syscall_cycles: 0,
                });
                return;
            }
            waiting_processes.push_back(process);
        }

        *processes = waiting_processes;
    }
}

impl ProcessMeta {
    pub fn new(pid: Pid, priority: i8, creation_time: usize) -> Self {
        Self {
            pid,
            priority,
            last_update: creation_time,
            state: ProcessState::Ready,
            timings: (0, 0, 0),
        }
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

/// Information about the process that is currently using the CPU.
pub struct CurrentProcessMeta<T> {
    /// The current process.
    pub process: T,
    /// The cycles that the process has executed on this run.
    pub execution_cycles: usize,
    /// The cycles that the process has spent in syscalls on this run.
    pub syscall_cycles: usize,
    /// The time this process has left before it is preempted.
    pub remaining_timeslice: usize,
}
pub struct ProcessManager<T> {
    pub timeslice_factor: usize,
    /// The maximum timeslice that is given to each process.
    pub timeslice: NonZeroUsize,
    /// The minimum timeslice that a process must have left
    /// after a syscall in order to remain on the CPU.
    pub minimum_remaining_timeslice: usize,
    /// Wether the scheduler will panic on the next query.
    pub will_panic: bool,
    /// The maximum pid that has been assigned yet.
    pub max_pid: usize,
    pub processes: VecDeque<T>,
    pub sleeping_processes: VecDeque<(T, usize)>,
    pub waiting_processes: HashMap<usize, VecDeque<T>>,
    /// The process that is currently running (or None if there isn't one).
    pub current_process: Option<CurrentProcessMeta<T>>,
    /// The number of clock cycles that have passed since the scheduler was started.
    pub clock: usize,
}

impl<T> ProcessManager<T>
where
    T: ProcessInformation + Send,
{
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self {
            timeslice_factor: 1,
            minimum_remaining_timeslice,
            processes: VecDeque::new(),
            sleeping_processes: VecDeque::new(),
            waiting_processes: HashMap::new(),
            current_process: None,
            will_panic: false,
            max_pid: 0,
            timeslice,
            clock: 0,
        }
    }

    pub fn get_next_process(&mut self) -> SchedulingDecision {
        self.update_timings();
        self.wake_processes();

        if self.will_panic {
            return SchedulingDecision::Panic;
        }

        T::get_next_process(
            &mut self.processes,
            &mut self.current_process,
            &mut self.timeslice_factor,
            self.timeslice,
        );

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

            SchedulingDecision::Sleep(NonZeroUsize::new(wait_interval).unwrap())
        } else {
            // There are no processes to be awaken. If there still
            // are processes waiting, signal a deadlock.
            if !self.waiting_processes.is_empty() {
                SchedulingDecision::Deadlock
            } else {
                SchedulingDecision::Done
            }
        }
    }

    pub fn get_processes(&self) -> impl Iterator<Item = &T> {
        let mut processes = Vec::new();

        if let Some(current) = &self.current_process {
            processes.push(current.process.as_process());
        }
        self.processes
            .iter()
            .chain(self.current_process.iter().map(|x| &x.process))
            .chain(self.sleeping_processes.iter().map(|(x, _)| x))
            .chain(self.waiting_processes.values().flatten())
    }

    /// Wake up all processes that finished waiting.
    pub fn wake_processes(&mut self) {
        let sleeping = self.sleeping_processes.drain(..);
        let (awaken, sleeping) = sleeping.partition(|(_, wake_time)| *wake_time <= self.clock);
        self.sleeping_processes = sleeping;

        let awaken = awaken.into_iter().map(|(mut process, _)| {
            process.set_state(ProcessState::Ready);
            process
        });

        self.processes.extend(awaken);
    }

    pub fn update_timings(&mut self) {
        let processes = self.processes.iter_mut();
        let waiting_processes = self.waiting_processes.values_mut().flatten();

        let sleeping_processes = self.sleeping_processes.iter_mut();
        let sleeping_processes = sleeping_processes.map(|(process, _)| process);

        for i in processes.chain(sleeping_processes).chain(waiting_processes) {
            let elapsed_time = self.clock - i.last_update();
            i.set_last_update(self.clock);
            i.add_total_time(elapsed_time);
        }
    }

    fn update_execution_time(&mut self) {
        if let Some(current) = self.current_process.as_mut() {
            let execution_time = self.clock - current.process.last_update();
            if execution_time > current.syscall_cycles {
                current.execution_cycles += execution_time - current.syscall_cycles;
            }

            current.process.add_execution_time(current.execution_cycles);
            current.process.set_last_update(self.clock);
        }
    }

    pub fn handle_process_stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall { syscall, remaining } => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice - remaining;
                    current.process.add_syscall();
                    current.syscall_cycles += 1;
                }

                let syscall_result = match syscall {
                    Fork(priority) => {
                        self.max_pid += 1;
                        let new_process = T::alloc(self, priority);
                        let new_pid = new_process.pid();
                        self.processes.push_back(new_process);

                        SyscallResult::Pid(new_pid)
                    }
                    Exit => {
                        if let Some(current) = self.current_process.take() {
                            // Killing `init` while other processes are
                            // running will result in a panic.
                            if current.process.pid() == 1
                                && (!self.processes.is_empty()
                                    || !self.sleeping_processes.is_empty()
                                    || !self.waiting_processes.is_empty())
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
                                    process.set_state(ProcessState::Ready);
                                    process
                                });

                            self.processes.extend(waiting_processes);
                        }
                        SyscallResult::Success
                    }
                    Wait(event) => {
                        self.update_execution_time();

                        if let Some(mut current) = self.current_process.take() {
                            current
                                .process
                                .set_state(ProcessState::Waiting { event: Some(event) });

                            self.waiting_processes
                                .entry(event)
                                .or_default()
                                .push_back(current.process);
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Sleep(time) => {
                        self.update_execution_time();

                        if let Some(CurrentProcessMeta { mut process, .. }) =
                            self.current_process.take()
                        {
                            process.set_state(ProcessState::Waiting { event: None });

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
                    if process.state() == ProcessState::Running {
                        let execution_time = *remaining_timeslice - remaining;
                        if execution_time > *syscall_cycles {
                            *execution_cycles += execution_time - *syscall_cycles;
                        }
                        *remaining_timeslice = remaining;
                        process.set_last_update(self.clock);
                    }
                }
                self.update_timings();

                if let Some(current) = self.current_process.as_mut() {
                    if current.remaining_timeslice < self.minimum_remaining_timeslice {
                        current.process.set_state(ProcessState::Ready);
                        current.process.add_execution_time(current.execution_cycles);
                    }
                }
                syscall_result
            }
            StopReason::Expired => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice;
                    current.process.set_state(ProcessState::Ready);
                    current
                        .process
                        .add_execution_time(current.remaining_timeslice);
                    current.process.set_last_update(self.clock);

                    self.update_timings();

                    SyscallResult::Success
                } else {
                    SyscallResult::NoRunningProcess
                }
            }
        }
    }
}
