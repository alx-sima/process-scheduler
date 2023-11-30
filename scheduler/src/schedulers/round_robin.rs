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
            current_process: None,
            max_pid: 0,
            clock: 0,
            will_panic: false,
        }
    }
}

impl Scheduler for RoundRobin {
    fn next(&mut self) -> SchedulingDecision {
        if self.will_panic {
            return SchedulingDecision::Panic;
        }
        if let Some(current) = &self.current_process {
            return SchedulingDecision::Run {
                pid: current.process.pid(),
                timeslice: NonZeroUsize::new(current.remaining_timeslice).unwrap(),
            };
        }

        println!("t {:?}", self.processes);

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
        }
        SchedulingDecision::Done
    }

    fn stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall { syscall, remaining } => {
                if let Some(current) = self.current_process.as_ref() {
                    println!("{} {}", current.remaining_timeslice, remaining);
                    self.clock += current.remaining_timeslice - remaining;
                }
                println!("clock {:?}", self.clock);
                // self.clock+= 1;
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
                            // self.clock += current.remaining_timeslice - remaining;
                            // self.clock -= 1;
                            // update(&mut self.processes, self.clock);
                            if current.process.pid() == 1 && self.processes.len() != 0 {
                                self.will_panic = true;
                            }
                            SyscallResult::Success
                        } else {
                            SyscallResult::NoRunningProcess
                        }
                    }
                    Signal(event) => {
                        todo!()
                        // for proc in &mut self.processes {
                        //     if let ProcessState::Waiting { event: Some(proc_event) } = proc.state {
                        //         if proc_event == event {
                        //             proc.state = ProcessState::Ready;
                        //         }
                        //     }
                        // }

                        // SyscallResult::Success
                    }
                    Wait(event) => {
                        todo!()
                        // if let Some(mut process) = self.processes.pop_front() {
                        //     process.state = ProcessState::Waiting { event: Some(event) };
                        //     process.timings.0 += self.timeslice.get() - remaining;

                        //     self.processes.push_back(process);
                        //     SyscallResult::Success
                        // } else {
                        //     SyscallResult::NoRunningProcess
                        // }
                    }
                    Sleep(time) => {
                        todo!()
                        // if let Some(mut process) = self.processes.pop_front() {
                        //     process.state = ProcessState::Waiting { event: None };
                        //     process.timings.0 += self.timeslice.get() - remaining;

                        //     self.sleep_timer.sleeping_processes.push_back((process, time));
                        //     SyscallResult::Success
                        // } else {
                        //     SyscallResult::NoRunningProcess
                        // }
                    }
                };
                // Update the sleep timer.
                if let Some(CurrentProcessMeta {
                    schedule_time: _,
                    execution_cycles,
                    syscall_cycles,
                    remaining_timeslice,
                    process,
                    ..
                }) = self.current_process.as_mut()
                {
                    *syscall_cycles += 1;
                    process.timings.0 += 1;
                    process.timings.1 += 1;
                    println!("{} {} {}", *remaining_timeslice, *syscall_cycles, remaining);
                    *execution_cycles += *remaining_timeslice - *syscall_cycles - remaining;
                    *remaining_timeslice = remaining;
                    process.last_update = self.clock;
                }

                update(&mut self.processes, self.clock);

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

                        // current.process.timings.0 += self.clock - current.schedule_time;
                        // current.process.timings.2 += current.execution_cycles;
                        // current.process.last_update = self.clock;
                        println!("test");
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

                    update(&mut self.processes, self.clock);
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
        ]
        .concat()
    }
}

fn fork(pid: usize, creation_time: usize) -> ProcessMeta {
    let new_process = ProcessMeta::new(Pid::new(pid), creation_time);
    new_process
}

fn update(p: &mut VecDeque<ProcessMeta>, time: usize) {
    for i in p.iter_mut() {
        i.timings.0 += time - i.last_update;
        println!("{}: {}", i.pid(), time - i.last_update);
        i.last_update = time;
    }
    println!("clock {} update {:?}", time, p);
}
