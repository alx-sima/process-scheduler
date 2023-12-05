use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
};

use crate::{Process, ProcessState, SchedulingDecision, StopReason,
            Syscall::{Exit, Fork, Signal, Sleep, Wait},
            SyscallResult,
};

/// A trait that provides actions needed by the scheduler for a process.
pub trait ProcessInformation: Process + Sized {
    /// Create a new process.
    ///
    /// @param scheduler    The scheduler's state when creating the process.
    /// @param priority     The priority of the process.
    fn create_process(scheduler: &ProcessManager<Self>, priority: i8) -> Self;

    /// Choose the next process to run, and stores it in `current_process`.
    /// If there is no process to run, `current_process` is set to `None`.
    ///
    /// @param processes                    The list of processes waiting to be run.
    /// @param[in,out] current_process      The process that is currently running.
    /// @param[in,out] timeslice_factor     The factor that is used to calculate the timeslice.
    /// @param timeslice                    The maximum timeslice that is given to each process.
    fn next_process(
        processes: &mut VecDeque<Self>,
        current_process: &mut Option<CurrentProcessMeta<Self>>,
        timeslice_factor: &mut usize,
        timeslice: NonZeroUsize,
    );

    /// Upcast the process to a `Process`.
    fn as_process(&self) -> &dyn Process;

    /// Set the state of the process to `state`.
    fn set_state(&mut self, state: ProcessState);

    /// Get the last update time of the process.
    fn last_update(&self) -> usize;

    /// Set the last update time of the process.
    fn set_last_update(&mut self, last_update: usize);

    /// Add `time` to the total time of the process.
    fn add_total_time(&mut self, time: usize);

    /// Add `time` to the execution (and total) time of the process.
    fn add_execution_time(&mut self, time: usize);

    /// Increase the time the process has spent in syscalls by 1.
    /// This also increases the total time of the process.
    fn add_syscall(&mut self);

    /// Increase the priority of the process by 1 (if possible).
    /// The priority can't be higher than the initial priority.
    fn increase_priority(&mut self) {}

    /// Decrease the priority of the process by 1 (if possible).
    /// The priority can't be lower than 0.
    fn decrease_priority(&mut self) {}

    /// Get the virtual runtime of the process (if implemented).
    /// If the scheduling algorithm doesn't implement this, it will always be 0.
    fn vruntime(&self) -> usize {
        0
    }
}

/// Information about the process that is currently using the CPU.
pub struct CurrentProcessMeta<T> {
    /// The current process.
    pub(super) process: T,
    /// The cycles that the process has executed on this run.
    pub(super) execution_cycles: usize,
    /// The cycles that the process has spent in syscalls on this run.
    pub(super) syscall_cycles: usize,
    /// The time this process has left before it is preempted.
    pub(super) remaining_timeslice: usize,
}

/// The process manager that keeps track of all processes and their state.
pub struct ProcessManager<T> {
    /// The process that is currently running (or None if there isn't one).
    pub(super) current_process: Option<CurrentProcessMeta<T>>,

    /// The processes that are ready to be run.
    pub(super) processes: VecDeque<T>,
    /// The processes that are sleeping for a certain event.
    /// The first element of the tuple is the process, the
    /// second is the time until it needs to sleep.
    pub(super) sleeping_processes: VecDeque<(T, usize)>,
    /// The processes that are waiting for a certain event.
    /// The key is the event, the value is a list of processes
    /// that are waiting for that event.
    pub(super) waiting_processes: HashMap<usize, VecDeque<T>>,

    /// The maximum timeslice that is given to each process.
    pub(super) timeslice: NonZeroUsize,
    /// The minimum timeslice that a process must have left
    /// after a syscall in order to remain on the CPU.
    pub(super) minimum_remaining_timeslice: usize,
    /// The factor that is used to calculate the timeslice (for cfs).
    pub(super) timeslice_factor: usize,

    /// The number of clock cycles that have passed since the scheduler was started.
    pub(super) clock: usize,
    /// The maximum pid that has been assigned yet.
    pub(super) max_pid: usize,
    /// Whether the scheduler will panic on the next query.
    pub(super) will_panic: bool,
}

impl<T> ProcessManager<T>
    where
        T: ProcessInformation + Send,
{
    /// Create a new process manager.
    ///
    /// @param timeslice                     The maximum timeslice that is given to each process.
    /// @param minimum_remaining_timeslice   The minimum timeslice that a process must have left
    pub fn new(timeslice: NonZeroUsize, minimum_remaining_timeslice: usize) -> Self {
        Self {
            minimum_remaining_timeslice,
            timeslice,
            sleeping_processes: VecDeque::new(),
            waiting_processes: HashMap::new(),
            processes: VecDeque::new(),
            current_process: None,
            timeslice_factor: 1,
            will_panic: false,
            max_pid: 0,
            clock: 0,
        }
    }

    /// Tell the CPU what to do next.
    ///
    /// @return The decision that the CPU should make.
    pub fn get_next_process(&mut self) -> SchedulingDecision {
        self.update_timings();
        self.wake_processes();

        if self.will_panic {
            return SchedulingDecision::Panic;
        }

        T::next_process(
            &mut self.processes,
            &mut self.current_process,
            &mut self.timeslice_factor,
            self.timeslice,
        );

        if let Some(current) = &self.current_process {
            if let Some(timeslice) = NonZeroUsize::new(current.remaining_timeslice) {
                return SchedulingDecision::Run {
                    pid: current.process.pid(),
                    timeslice,
                };
            }
        }

        // There aren't any ready processes, wait for the first to wake up.
        if let Some(wait_time) = self
            .sleeping_processes
            .iter()
            .filter_map(|(_, wake_time)| NonZeroUsize::new(wake_time - self.clock))
            .min()
        {
            self.clock += wait_time.get();
            SchedulingDecision::Sleep(wait_time)
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

    /// Handle an interrupt from the CPU.
    ///
    /// @param reason   The reason why the CPU stopped executing the
    ///                 process (and called the scheduler).
    /// @return The result of the syscall.
    pub fn handle_process_stop(&mut self, reason: StopReason) -> SyscallResult {
        match reason {
            StopReason::Syscall { syscall, remaining } => {
                if let Some(current) = self.current_process.as_mut() {
                    self.clock += current.remaining_timeslice - remaining;
                    let execution_time = current.remaining_timeslice - remaining - 1;

                    current.process.add_execution_time(execution_time);
                    current.process.add_syscall();
                    current.process.set_last_update(self.clock);

                    current.syscall_cycles += 1;
                    current.remaining_timeslice = remaining;
                }

                let syscall_result = match syscall {
                    Fork(priority) => {
                        self.max_pid += 1;
                        let new_process = T::create_process(self, priority);
                        let new_pid = new_process.pid();
                        self.processes.push_back(new_process);

                        SyscallResult::Pid(new_pid)
                    }
                    Exit => {
                        if let Some(current) = self.current_process.take() {
                            // Killing `init` while other processes are
                            // running will result in a panic.
                            if current.process.pid() == 1
                                && !(self.processes.is_empty()
                                && self.sleeping_processes.is_empty()
                                && self.waiting_processes.is_empty())
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

                // Preempt the process if it doesn't have enough time left.
                if let Some(current) = self.current_process.as_mut()
                {
                    if current.remaining_timeslice < self.minimum_remaining_timeslice {
                        current.process.set_state(ProcessState::Ready);
                        current.process.add_execution_time(current.execution_cycles);
                    }
                }

                syscall_result
            }
            StopReason::Expired => {
                if let Some(current) = self.current_process.as_mut() {
                    let process = &mut current.process;
                    self.clock += current.remaining_timeslice;

                    process.set_state(ProcessState::Ready);
                    process.add_execution_time(current.remaining_timeslice);
                    process.set_last_update(self.clock);

                    self.update_timings();

                    SyscallResult::Success
                } else {
                    SyscallResult::NoRunningProcess
                }
            }
        }
    }

    /// Update the execution time of the current process.
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

    /// Update the timings of all processes.
    fn update_timings(&mut self) {
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

    /// Wake up all processes that finished waiting.
    fn wake_processes(&mut self) {
        let sleeping = self.sleeping_processes.drain(..);
        let (awaken, sleeping) = sleeping.partition(|(_, wake_time)| *wake_time <= self.clock);
        self.sleeping_processes = sleeping;

        let awaken = awaken.into_iter().map(|(mut process, _)| {
            process.set_state(ProcessState::Ready);
            process
        });

        self.processes.extend(awaken);
    }

    /// Get an iterator over all processes.
    pub(super) fn get_processes(&self) -> impl Iterator<Item=&T> {
        self.processes
            .iter()
            .chain(self.current_process.iter().map(|current| &current.process))
            .chain(self.sleeping_processes.iter().map(|(process, _)| process))
            .chain(self.waiting_processes.values().flatten())
    }
}
