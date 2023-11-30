use std::{collections::VecDeque, num::NonZeroUsize};

use crate::{
    Pid, Process, ProcessState, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};
#[derive(Debug)]
pub struct ProcessMeta {
    pid: Pid,
    state: ProcessState,
    last_update: usize,
    priority: i8,
    timings: (
        // Total
        usize,
        // Syscall
        usize,
        // Execution
        usize,
    ),
}

impl ProcessMeta {
    fn new(pid: Pid, priority: i8, creation_time: usize) -> Self {
        ProcessMeta {
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

#[derive(Debug)]
pub struct CurrentProcessMeta {
    pub process: ProcessMeta,
    pub execution_cycles: usize,
    pub remaining_timeslice: usize,
    pub syscall_cycles: usize,
}
pub struct ProcessManager {
    pub timeslice: NonZeroUsize,
    pub minimum_remaining_timeslice: usize,
    pub will_panic: bool,
    pub max_pid: usize,
    pub processes: VecDeque<ProcessMeta>,
    pub sleep_timer: VecDeque<(ProcessMeta, usize)>,
    /// The process that is currently running (or None if there isn't one).
    pub current_process: Option<CurrentProcessMeta>,
    /// The number of clock cycles that have passed since the scheduler was started.
    pub clock: usize,
}

impl ProcessManager {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self {
            minimum_remaining_timeslice,
            processes: VecDeque::new(),
            sleep_timer: VecDeque::new(),
            current_process: None,
            will_panic: false,
            max_pid: 0,
            timeslice,
            clock: 0,
        }
    }

    pub fn get_processes(&self) -> Vec<&ProcessMeta> {
        self.processes
            .iter()
            .chain(self.sleep_timer.iter().map(|(x, _)| x))
            .collect()
    }

    /// Wake up all processes that finished waiting.
    pub fn wake_processes(&mut self) {
        let sleeping = self.sleep_timer.drain(..);
        let (awaken, sleeping) = sleeping.partition(|(_, wake_time)| *wake_time <= self.clock);
        self.sleep_timer = sleeping;

        let awaken = awaken.into_iter().map(|(process, _)| ProcessMeta {
            state: ProcessState::Ready,
            ..process
        });
        self.processes.extend(awaken);
    }

    pub fn get_next_process(&mut self) -> Option<CurrentProcessMeta> {
        let mut waiting_processes = VecDeque::new();

        while let Some(mut process) = self.processes.pop_front() {
            if process.state == ProcessState::Ready {
                process.state = ProcessState::Running;
                self.processes.extend(waiting_processes);
                return Some(CurrentProcessMeta {
                    process,
                    remaining_timeslice: self.timeslice.get(),
                    execution_cycles: 0,
                    syscall_cycles: 0,
                });
            }
            waiting_processes.push_back(process);
        }
        self.processes = waiting_processes;
        None
    }

    pub fn update_timings(&mut self) {
        let p = &mut self.processes;
        let s = &mut self.sleep_timer;
        let time = self.clock;
        let s = s.into_iter().map(|(process, _)| process);
        for i in p.iter_mut().chain(s) {
            let elapsed_time = time - i.last_update;
            i.last_update = time;

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
                    println!("{} {}", current.remaining_timeslice, remaining);
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
                                && (self.processes.len() != 0 || self.sleep_timer.len() != 0)
                            {
                                self.will_panic = true;
                            }

                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Signal(event) => {
                        for proc in &mut self.processes {
                            if let ProcessState::Waiting {
                                event: Some(proc_event),
                            } = proc.state
                            {
                                if proc_event == event {
                                    proc.state = ProcessState::Ready;
                                }
                            }
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
                            self.processes.push_back(current.process);
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

                            self.sleep_timer.push_back((process, self.clock + time));
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
                    let execution_time = *remaining_timeslice - remaining;
                    if execution_time > *syscall_cycles {
                        *execution_cycles += execution_time - *syscall_cycles;
                    }
                    *remaining_timeslice = remaining;
                    process.last_update = self.clock;
                }
                self.update_timings();

                if let Some(mut current) = self.current_process.take() {
                    if current.remaining_timeslice < self.minimum_remaining_timeslice {
                        current.process.state = ProcessState::Ready;
                        current.process.timings.0 += current.execution_cycles;
                        current.process.timings.2 += current.execution_cycles;

                        self.processes.push_back(current.process);
                    } else {
                        self.current_process = Some(current);
                    }
                }
                syscall_result
            }
            StopReason::Expired => {
                if let Some(mut current) = self.current_process.take() {
                    self.clock += current.remaining_timeslice;
                    current.process.state = ProcessState::Ready;
                    current.process.timings.0 += current.remaining_timeslice;
                    current.process.timings.2 += current.remaining_timeslice;
                    current.process.last_update = self.clock;

                    self.update_timings();
                    self.processes.push_back(current.process);

                    SyscallResult::Success
                } else {
                    SyscallResult::NoRunningProcess
                }
            }
        }
    }
}
