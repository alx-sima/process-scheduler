use crate::{
    Pid, Process, ProcessState, Scheduler, SchedulingDecision, StopReason,
    Syscall::{Exit, Fork, Signal, Sleep, Wait},
    SyscallResult,
};

use std::{collections::VecDeque, num::NonZeroUsize};

#[derive(Debug)]
struct ProcessMeta {
    pid: Pid,
    state: ProcessState,
    last_update: usize,
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
    fn new(pid: Pid, creation_time: usize) -> Self {
        ProcessMeta {
            pid,
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
        0
    }

    fn extra(&self) -> String {
        String::new()
    }
}

#[derive(Debug)]
struct CurrentProcessMeta {
    process: ProcessMeta,
    execution_cycles: usize,
    remaining_timeslice: usize,
    syscall_cycles: usize,
    schedule_time: usize,
}

pub struct RoundRobin {
    timeslice: NonZeroUsize,
    minimum_remaining_timeslice: usize,
    will_panic: bool,
    max_pid: usize,
    processes: VecDeque<ProcessMeta>,
    sleep_timer: VecDeque<(ProcessMeta, usize)>,
    /// The process that is currently running (or None if there isn't one).
    current_process: Option<CurrentProcessMeta>,
    /// The number of clock cycles that have passed since the scheduler was started.
    clock: usize,
}

impl RoundRobin {
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        RoundRobin {
            timeslice,
            minimum_remaining_timeslice,
            processes: VecDeque::new(),
            sleep_timer: VecDeque::new(),
            current_process: None,
            max_pid: 0,
            clock: 0,
            will_panic: false,
        }
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> SchedulingDecision {
        update(&mut self.processes, &mut self.sleep_timer, self.clock);
        if self.will_panic {
            return SchedulingDecision::Panic;
        }
        if let Some(current) = &self.current_process {
            return SchedulingDecision::Run {
                pid: current.process.pid(),
                timeslice: NonZeroUsize::new(current.remaining_timeslice).unwrap(),
            };
        }

        // loop {
        let (awaken, sleeping): (VecDeque<_>, VecDeque<_>) = self
            .sleep_timer
            .drain(..)
            .partition(|(_, wake_time)| *wake_time <= self.clock);
        self.sleep_timer = sleeping;

        let awaken = awaken.into_iter().map(|(process, _)| ProcessMeta {
            state: ProcessState::Ready,
            ..process
        });
        self.processes.extend(awaken);

        if let Some(index) = self.processes.iter().enumerate().find_map(|(index, x)| {
            if x.state == ProcessState::Ready {
                Some(index)
            } else {
                None
            }
        }) {
            if let Some(mut process) = self.processes.remove(index) {
                process.state = ProcessState::Running;
                let pid = process.pid();

                self.current_process = Some(CurrentProcessMeta {
                    process,
                    remaining_timeslice: self.timeslice.get(),
                    execution_cycles: 0,
                    syscall_cycles: 0,
                    schedule_time: self.clock,
                });

                return SchedulingDecision::Run {
                    pid,
                    timeslice: self.timeslice,
                };
            }
        } else {
            // There aren't any ready processes, wait for the first to wake up.
            if let Some(first_wake) = self
                .sleep_timer
                .iter()
                .map(|(_, wake_time)| wake_time)
                .min()
            {
                let wait_interval = first_wake - self.clock;
                println!("wait for {}", wait_interval);
                self.clock = *first_wake;

                return SchedulingDecision::Sleep(NonZeroUsize::new(wait_interval).unwrap());
            } else {
                // No process is ready to wake up.
                return SchedulingDecision::Done;
            }
        }
        // }
        SchedulingDecision::Done
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall { syscall, remaining } => {
                if let Some(current) = self.current_process.as_mut() {
                    println!("{} {}", current.remaining_timeslice, remaining);
                    self.clock += current.remaining_timeslice - remaining;
                    current.process.timings.0 += 1;
                    current.process.timings.1 += 1;
                    current.syscall_cycles += 1;
                }
                println!("clock {:?}", self.clock);

                let syscall_result = match syscall {
                    Fork(_) => {
                        self.max_pid += 1;
                        let new_process = fork(self.max_pid, self.clock);
                        let new_pid = new_process.pid();
                        self.processes.push_back(new_process);

                        SyscallResult::Pid(new_pid)
                    }
                    Exit => {
                        if let Some(current) = self.current_process.take() {
                            // Killing `init` while other processes are
                            // running will result in a panic.
                            if current.process.pid() == 1 && self.processes.len() != 0 {
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
                            current.execution_cycles +=
                                self.clock - current.process.last_update - current.syscall_cycles;
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

                            print!("sleeping until{}", self.clock + time);
                            self.sleep_timer.push_back((process, self.clock + time));
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                };

                // Update the timer.
                if let Some(CurrentProcessMeta {
                    schedule_time: _,
                    execution_cycles,
                    syscall_cycles,
                    remaining_timeslice,
                    process,
                    ..
                }) = self.current_process.as_mut()
                {
                    println!("{} {} {}", *remaining_timeslice, *syscall_cycles, remaining);
                    *execution_cycles += *remaining_timeslice - (*syscall_cycles + remaining);
                    *remaining_timeslice = remaining;
                    process.last_update = self.clock;
                }
                update(&mut self.processes, &mut self.sleep_timer, self.clock);

                if let Some(mut current) = self.current_process.take() {
                    if current.remaining_timeslice < self.minimum_remaining_timeslice {
                        current.process.state = ProcessState::Ready;
                        current.process.timings.0 += current.execution_cycles;
                        current.process.timings.2 += current.execution_cycles;
                        println!(
                            "Debug: Execution cycles for PID {}: {}",
                            current.process.pid(),
                            current.execution_cycles
                        );
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

                    update(&mut self.processes, &mut self.sleep_timer, self.clock);
                    self.processes.push_back(current.process);

                    SyscallResult::Success
                } else {
                    SyscallResult::NoRunningProcess
                }
            }
        }
    }

    fn list(&mut self) -> Vec<&dyn Process> {
        let mut processes = Vec::new();

        if let Some(current) = &self.current_process {
            processes.push(&current.process as &dyn Process);
        }

        [
            processes,
            self.processes.iter().map(|x| x as &dyn Process).collect(),
            self.sleep_timer
                .iter()
                .map(|(x, _)| x as &dyn Process)
                .collect(),
        ]
        .concat()
    }
}

fn fork(pid: usize, creation_time: usize) -> ProcessMeta {
    let new_process = ProcessMeta::new(Pid::new(pid), creation_time);
    new_process
}

fn update(p: &mut VecDeque<ProcessMeta>, s: &mut VecDeque<(ProcessMeta, usize)>, time: usize) {
    let s = s.into_iter().map(|(process, _)| process);
    for i in p.iter_mut().chain(s) {
        i.timings.0 += time - i.last_update;
        println!("{}: {}", i.pid(), time - i.last_update);
        println!("updated {}", i.last_update);
        i.last_update = time;
    }
    println!("clock {} update {:?}", time, p);
}
