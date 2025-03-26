use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use postcard::accumulator::{CobsAccumulator, FeedResult};
use poststation_sdk::connect_insecure;
use template_icd::{Event, TraceReportTopic};

use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<(), String> {
    tokio::task::spawn(inner_main()).await.unwrap().unwrap();
    Ok(())
}

#[derive(Clone, Debug)]
enum TaskState {
    Idle { since: u64 },
    Waiting { since: u64 },
    Active { since: u64, pended: bool },
}

#[derive(Clone, Debug, Default)]
struct TaskTime {
    ticks_active: u64,
    ticks_waiting: u64,
    ticks_idle: u64,
}

#[derive(Clone, Debug)]
struct TaskData {
    name: String,
    readies: usize,
    time: TaskTime,
    state: TaskState,
}

impl TaskData {
    fn new_idle(task_id: u32, tick: u64) -> Self {
        Self {
            name: format!("T-{task_id:08X}"),
            readies: 0,
            time: TaskTime::default(),
            state: TaskState::Idle { since: tick },
        }
    }

    fn new_waiting(task_id: u32, tick: u64) -> Self {
        Self {
            name: format!("T-{task_id:08X}"),
            readies: 0,
            time: TaskTime::default(),
            state: TaskState::Waiting { since: tick },
        }
    }

    fn new_active(task_id: u32, tick: u64) -> Self {
        Self {
            name: format!("T-{task_id:08X}"),
            readies: 0,
            time: TaskTime::default(),
            state: TaskState::Active {
                since: tick,
                pended: false,
            },
        }
    }

    fn set_idle(&mut self, tick: u64) {
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

    fn set_waiting(&mut self, tick: u64) {
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

    fn set_active(&mut self, tick: u64) {
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

    fn take_report(&mut self, now: u64) -> (usize, TaskTime) {
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

        (readies, time)
    }
}

#[derive(Debug, Default)]
struct System {
    tasks: HashMap<u32, TaskData>,
    exec_last_idle: Option<u64>,
    exec_last_active: Option<u64>,
    exec_ticks_idle: u64,
    exec_ticks_active: u64,
    current_tick: u64,
    last_delta: Option<u64>,
    last_render: Option<Instant>,
    bytes: usize,
}

impl System {
    fn handle_evt(&mut self, evt: Event, bytes: usize) {
        let last_delta = *match self.last_delta.as_mut() {
            None => {
                if let Event::ExecutorPollStart { tick } = evt {
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
                if let Some(t) = self.exec_last_active.take() {
                    let delta = tick.checked_sub(t).unwrap();
                    self.exec_ticks_active += delta;
                }
                self.exec_last_idle = Some(tick);
                // println!("exec_last_idle: {exec_last_idle:?}");
            }
            Event::ExecutorPollStart { tick } => {
                // NOTE: uses abs time, sets delta time
                self.last_delta = Some(tick);
                self.current_tick = tick;
                if let Some(t) = self.exec_last_idle.take() {
                    let delta = tick.checked_sub(t).unwrap();
                    self.exec_ticks_idle += delta;
                }
                self.exec_last_active = Some(tick);
                // println!("exec_last_active: {exec_last_active:?}");
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
        }
    }

    fn check_render(&mut self) {
        let last = *self.last_render.get_or_insert_with(Instant::now);
        if last.elapsed() > Duration::from_secs(1) {
            self.last_render = Some(Instant::now());
            let mut vec = vec![];
            for (k, v) in self.tasks.iter_mut() {
                let (readies, time) = v.take_report(self.current_tick);
                vec.push((*k, readies, &v.name, time));
            }
            vec.sort_by_key(|(k, _r, _n, _t)| *k);
            let ttl_exec_ticks = self.exec_ticks_idle + self.exec_ticks_active;
            let ttl_exec_ticks = ttl_exec_ticks as f64;

            println!();
            println!("KiB/s: {:.02}", self.bytes as f64 / 1024.0);
            println!(
                "EXECUTOR:   IDLE: {:>6.02}%, ACTV: {:>6.02}%",
                100.0 * self.exec_ticks_idle as f64 / ttl_exec_ticks,
                100.0 * self.exec_ticks_active as f64 / ttl_exec_ticks,
            );
            self.exec_ticks_idle = 0;
            self.exec_ticks_active = 0;
            self.bytes = 0;

            for (_k, readies, name, time) in vec {
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
                    print!("WAKE: {}", readies);
                }

                println!();
            }
        }
    }
}

async fn inner_main() -> Result<(), String> {
    const SERIAL: u64 = 0x4274E020EBB2D27E;
    let client = connect_insecure(51837).await.unwrap();
    let mut sub = client
        .stream_topic::<TraceReportTopic>(SERIAL)
        .await
        .unwrap();
    let mut system = System::default();

    let mut acc = CobsAccumulator::<64>::new();

    loop {
        let msg = sub.recv().await.unwrap();

        let mut window = msg.events_cobs.as_slice();
        'cobs: while !window.is_empty() {
            window = match acc.feed_ref::<Event>(window) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(_new_wind) => todo!(),
                FeedResult::DeserError(_new_wind) => todo!(),
                FeedResult::Success {
                    data: evt,
                    remaining,
                } => {
                    system.handle_evt(evt, window.len() - remaining.len());
                    remaining
                }
            };
        }

        system.check_render();
    }
}
