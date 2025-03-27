use std::{collections::HashMap, time::{Duration, Instant}};

use template_icd::Event;


#[derive(Clone, Debug)]
pub struct ExecData {
    pub state: ExecState,
    pub time: ExecTime,
}

impl ExecData {
    pub fn set_idle(&mut self, tick: u64) {
        match self.state {
            ExecState::Idle { .. } => println!("WARN: eidle -> eidle?"),
            ExecState::Scheduling { since } => {
                let delta = tick - since;
                self.time.ticks_sched += delta;
                self.state = ExecState::Idle { since: tick };
            },
            ExecState::Polling { since } => {
                let delta = tick - since;
                self.time.ticks_poll += delta;
                self.state = ExecState::Idle { since: tick };
            },
        }
    }

    pub fn set_sched(&mut self, tick: u64) {
        match self.state {
            ExecState::Idle { since } => {
                let delta = tick - since;
                self.time.ticks_idle += delta;
                self.state = ExecState::Scheduling { since: tick };
            },
            ExecState::Scheduling { .. } => println!("WARN: esched -> esched?"),
            ExecState::Polling { since } => {
                let delta = tick - since;
                self.time.ticks_poll += delta;
                self.state = ExecState::Scheduling { since: tick };
            },
        }
    }

    pub fn set_polling(&mut self, tick: u64) {
        match self.state {
            ExecState::Idle { since } => {
                let delta = tick - since;
                self.time.ticks_idle += delta;
                self.state = ExecState::Polling { since: tick };
            },
            ExecState::Scheduling { since } => {
                let delta = tick - since;
                self.time.ticks_sched += delta;
                self.state = ExecState::Polling { since: tick };
            },
            ExecState::Polling { .. } => println!("WARN: epoll -> epoll?"),
        }
    }

    pub fn take_report(&mut self, now: u64) -> ExecTime {
        match &mut self.state {
            ExecState::Idle { since } => {
                let s = core::mem::replace(since, now);
                let delta = now.checked_sub(s).unwrap();
                self.time.ticks_idle += delta;
            }
            ExecState::Scheduling { since } => {
                let s = core::mem::replace(since, now);
                let delta = now.checked_sub(s).unwrap();
                self.time.ticks_sched += delta;
            }
            ExecState::Polling { since } => {
                let s = core::mem::replace(since, now);
                let delta = now.checked_sub(s).unwrap();
                self.time.ticks_poll += delta;
            }
        }
        core::mem::take(&mut self.time)
    }
}

#[derive(Clone, Debug)]
pub enum ExecState {
    Idle { since: u64 },
    Scheduling { since: u64 },
    Polling { since: u64 },
}

#[derive(Clone, Debug, Default)]
pub struct ExecTime {
    pub ticks_idle: u64,
    pub ticks_sched: u64,
    pub ticks_poll: u64,
}

#[derive(Clone, Debug)]
pub enum TaskState {
    Idle { since: u64 },
    Waiting { since: u64 },
    Active { since: u64, pended: bool },
}

#[derive(Clone, Debug, Default)]
pub struct TaskTime {
    pub ticks_active: u64,
    pub ticks_waiting: u64,
    pub ticks_idle: u64,
}

#[derive(Clone, Debug, Default)]
pub struct DeadlineData {
    pub deadstate: DeadlineState,
    pub deadlines_retired: usize,
    pub deadlines_met: usize,
    pub deadlines_violated: usize,
    pub deadline_deviant_ticks: u64,
}

#[derive(Clone, Debug)]
pub struct TaskData {
    pub name: String,
    pub readies: usize,
    pub time: TaskTime,
    pub state: TaskState,
    pub dead: DeadlineData,
}

impl TaskData {
    pub fn new_idle(task_id: u32, tick: u64) -> Self {
        Self {
            name: format!("T-{task_id:08X}"),
            readies: 0,
            time: TaskTime::default(),
            state: TaskState::Idle { since: tick },
            dead: DeadlineData::default(),
        }
    }

    pub fn new_waiting(task_id: u32, tick: u64) -> Self {
        Self {
            name: format!("T-{task_id:08X}"),
            readies: 0,
            time: TaskTime::default(),
            state: TaskState::Waiting { since: tick },
            dead: DeadlineData::default(),
        }
    }

    pub fn new_active(task_id: u32, tick: u64) -> Self {
        Self {
            name: format!("T-{task_id:08X}"),
            readies: 0,
            time: TaskTime::default(),
            state: TaskState::Active {
                since: tick,
                pended: false,
            },
            dead: DeadlineData::default(),
        }
    }

    pub fn set_idle(&mut self, tick: u64) {
        // println!("  -> set_idle");
        let was_pended = match self.state {
            TaskState::Idle { .. } => false,
            TaskState::Waiting { .. } => {
                println!("WARN: waiting -> idle?");
                false
            }
            TaskState::Active { since, pended } => {
                let delta = tick.checked_sub(since).unwrap_or_else(|| {
                    panic!("{tick} - {since}");
                });
                self.time.ticks_active += delta;
                pended
            }
        };
        if was_pended {
            self.state = TaskState::Waiting { since: tick };
        } else {
            self.state = TaskState::Idle { since: tick };
        }
    }

    pub fn set_waiting(&mut self, tick: u64) {
        // println!("  -> set_waiting");
        match &mut self.state {
            TaskState::Idle { since } => {
                self.readies += 1;
                let delta = tick.checked_sub(*since).unwrap_or_else(|| {
                    panic!("{tick} - {since}");
                });
                self.time.ticks_idle += delta;
                self.state = TaskState::Waiting { since: tick };
            }
            TaskState::Waiting { .. } => {}
            TaskState::Active { since: _, pended } => {
                *pended = true;
            }
        }
    }

    pub fn set_active(&mut self, tick: u64) {
        // println!("  -> set_active");
        match self.state {
            TaskState::Idle { .. } => println!("WARN: idle -> active?"),
            TaskState::Waiting { since } => {
                let delta = tick.checked_sub(since).unwrap_or_else(|| {
                    panic!("{tick} - {since}");
                });
                self.time.ticks_waiting += delta;
            }
            TaskState::Active { .. } => {}
        }
        self.state = TaskState::Active {
            since: tick,
            pended: false,
        }
    }

    pub fn start_deadline(&mut self, now: u64, dur: u64) {
        match self.dead.deadstate {
            DeadlineState::Inactive => {
                self.dead.deadstate = DeadlineState::Active { at: now, deadline_at: now + dur };
            },
            DeadlineState::Active { .. } => {
                panic!("WARN: Deadline restart?");
            },
        }
    }

    pub fn stop_deadline(&mut self, now: u64) {
        match self.dead.deadstate {
            DeadlineState::Inactive => {},
            DeadlineState::Active { at: _, deadline_at } => {
                self.dead.deadlines_retired += 1;
                if now > deadline_at {
                    self.dead.deadlines_violated += 1;
                    self.dead.deadline_deviant_ticks += now - deadline_at;
                } else {
                    self.dead.deadlines_met += 1;
                }
                self.dead.deadstate = DeadlineState::Inactive;
            },
        }
    }

    pub fn take_report(&mut self, now: u64) -> (usize, TaskTime, DeadReport) {
        match &mut self.state {
            TaskState::Idle { since } => {
                let s = core::mem::replace(since, now);
                let delta = now.checked_sub(s).unwrap();
                self.time.ticks_idle += delta;
            }
            TaskState::Waiting { since } => {
                let s = core::mem::replace(since, now);
                let delta = now.checked_sub(s).unwrap();
                self.time.ticks_waiting += delta;
            }
            TaskState::Active { since, pended: _ } => {
                let s = core::mem::replace(since, now);
                let delta = now.checked_sub(s).unwrap();
                self.time.ticks_active += delta;
            }
        }
        let time = core::mem::take(&mut self.time);
        let readies = core::mem::take(&mut self.readies);
        let dead = DeadReport {
            deadlines_retired: core::mem::take(&mut self.dead.deadlines_retired),
            deadlines_met: core::mem::take(&mut self.dead.deadlines_met),
            deadlines_violated: core::mem::take(&mut self.dead.deadlines_violated),
            deadline_deviant_ticks: core::mem::take(&mut self.dead.deadline_deviant_ticks),
        };

        (readies, time, dead)
    }
}

#[derive(Debug, Clone, Default)]
pub enum DeadlineState {
    #[default]
    Inactive,
    Active { at: u64, deadline_at: u64 },
}

pub struct TaskReport<'a> {
    pub task_id: u32,
    pub readies: usize,
    pub name: &'a str,
    pub time: TaskTime,
    pub dead: DeadReport,
}

pub struct DeadReport {
    pub deadlines_retired: usize,
    pub deadlines_met: usize,
    pub deadlines_violated: usize,
    pub deadline_deviant_ticks: u64,
}

pub struct IntervalReport<'a> {
    pub tasks: Vec<TaskReport<'a>>,
    pub exec_time: ExecTime,
    pub bytes: usize,
}

#[derive(Debug)]
pub struct System {
    pub tasks: HashMap<u32, TaskData>,
    pub exec: ExecData,
    pub current_tick: u64,
    pub last_delta: Option<u64>,
    pub last_render: Option<Instant>,
    pub bytes: usize,
}

impl Default for System {
    fn default() -> Self {
        Self {
            tasks: Default::default(),
            exec: ExecData { state: ExecState::Idle { since: 0 }, time: Default::default() },
            current_tick: Default::default(),
            last_delta: Default::default(),
            last_render: Default::default(),
            bytes: Default::default(),
        }
    }
}

impl System {
    pub fn handle_evt(&mut self, evt: Event, bytes: usize) {
        let last_delta = *match self.last_delta.as_mut() {
            None => {
                if let Event::ExecutorPollStart { tick } = evt {
                    self.exec.state = ExecState::Polling { since: tick };
                    self.current_tick = tick;
                    self.last_delta.insert(tick)
                } else {
                    return;
                }
            }
            Some(ld) => ld,
        };
        self.bytes += bytes;

        // print!("{evt:?} => ");
        match evt {
            Event::TaskNew { .. } => {
                // NOTE: uses abs time
            }
            Event::TaskExecBegin { tick, task_id } => {
                // NOTE: uses delta time
                let tick = tick + last_delta;
                self.current_tick = tick;
                self.exec.set_polling(tick);
                // Ready -> Active
                let tsk = self
                    .tasks
                    .entry(task_id)
                    .or_insert_with(|| TaskData::new_active(task_id, tick));
                tsk.set_active(tick);
            }
            Event::TaskExecEnd { tick, task_id } => {
                // NOTE: uses delta time
                let tick = tick + last_delta;
                self.current_tick = tick;
                self.exec.set_sched(tick);
                // Active -> Idle
                let tsk = self
                    .tasks
                    .entry(task_id)
                    .or_insert_with(|| TaskData::new_idle(task_id, tick));
                tsk.set_idle(tick);
            }
            Event::TaskReadyBegin { tick, task_id } => {
                // NOTE: uses delta time
                let tick = tick + last_delta;
                self.current_tick = tick;
                // Idle -> Ready
                let tsk = self
                    .tasks
                    .entry(task_id)
                    .or_insert_with(|| TaskData::new_waiting(task_id, tick));
                tsk.set_waiting(tick);
            }
            Event::ExecutorIdle { tick } => {
                // NOTE: uses delta time
                let tick = tick + last_delta;
                self.current_tick = tick;
                self.exec.set_idle(tick);
            }
            Event::ExecutorPollStart { tick } => {
                // NOTE: uses abs time, sets delta time
                self.last_delta = Some(tick);
                self.current_tick = tick;
                self.exec.set_sched(tick);
            }
            Event::TaskIdentify { tick, task_id, name } => {
                let tick = tick + last_delta;
                self.current_tick = tick;
                if let Some(t) = self.tasks.get_mut(&task_id) {
                    if t.name.as_str() != name {
                        t.name = name.to_string();
                    }
                }
            }
            Event::DeadlineStart { tick, task_id, deadline } => {
                let tick = tick + last_delta;
                self.current_tick = tick;
                if let Some(t) = self.tasks.get_mut(&task_id) {
                    t.start_deadline(tick, deadline);
                }
            },
            Event::DeadlineStop { tick, task_id } => {
                let tick = tick + last_delta;
                self.current_tick = tick;
                if let Some(t) = self.tasks.get_mut(&task_id) {
                    t.stop_deadline(tick);
                }
            },
        }
    }

    pub fn capture_report(&mut self) -> IntervalReport<'_> {
        let mut vec = vec![];
        for (k, v) in self.tasks.iter_mut() {
            let (readies, time, dead) = v.take_report(self.current_tick);
            vec.push(TaskReport { task_id: *k, readies, name: &v.name, time, dead });
        }
        vec.sort_by_key(|tr| tr.task_id);

        let exec_rpt = self.exec.take_report(self.current_tick);
        let bytes = self.bytes;
        self.bytes = 0;
        IntervalReport { tasks: vec, exec_time: exec_rpt, bytes }
    }

    pub fn check_render(&mut self) {
        let last = *self.last_render.get_or_insert_with(Instant::now);
        if last.elapsed() > Duration::from_secs(1) {
            self.last_render = Some(Instant::now());
            self.render();
        }
    }

    pub fn render(&mut self) {
        let rpt = self.capture_report();

        let ttl_exec_ticks = rpt.exec_time.ticks_idle + rpt.exec_time.ticks_sched + rpt.exec_time.ticks_poll;
        let ttl_exec_ticks = ttl_exec_ticks as f64;

        println!();
        println!("KiB/s: {:.02}", rpt.bytes as f64 / 1024.0);
        println!(
            "EXECUTOR:   IDLE: {:>6.02}%, POLL: {:>6.02}%, SCHD: {:>6.02}%",
            100.0 * rpt.exec_time.ticks_idle as f64 / ttl_exec_ticks,
            100.0 * rpt.exec_time.ticks_poll as f64 / ttl_exec_ticks,
            100.0 * rpt.exec_time.ticks_sched as f64 / ttl_exec_ticks,
        );

        for TaskReport { task_id: _, readies, name, time, dead } in rpt.tasks {

            let ttl_ticks = time.ticks_idle + time.ticks_active + time.ticks_waiting;
            if ttl_ticks == 0 {
                println!("No ticks!")
            } else {
                print!("{name:>10}: ");
                let ttl_ticks = ttl_ticks as f64;
                print!(
                    "IDLE: {:>6.02}%, ",
                    100.0 * time.ticks_idle as f64 / ttl_ticks
                );
                print!(
                    "ACTV: {:>6.02}%, ",
                    100.0 * time.ticks_active as f64 / ttl_ticks
                );
                print!(
                    "WAIT: {:>6.02}%, ",
                    100.0 * time.ticks_waiting as f64 / ttl_ticks
                );
                print!("WAKE: {:04}, ", readies);
                print!("DRET: {:04}, ", dead.deadlines_retired);
                print!("DMET: {:04}, ", dead.deadlines_met);
                print!("DVIO: {:04}, ", dead.deadlines_violated);
                print!("DELT: {:04}", dead.deadline_deviant_ticks);
            }

            println!();
        }
    }
}
