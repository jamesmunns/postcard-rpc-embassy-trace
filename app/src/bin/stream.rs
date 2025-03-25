use std::{collections::HashMap, time::{Duration, Instant}, u64};

use poststation_sdk::{connect, connect_insecure};
use template_icd::{Event, TraceReportTopic};

use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<(), String> {
    tokio::task::spawn(inner_main()).await.unwrap().unwrap();
    Ok(())
}

#[derive(Default, Clone, Debug)]
struct TaskData {
    readies: usize,
    ticks_active: u64,
    ticks_waiting: u64,
    ticks_idle: u64,
    last_ready: Option<u64>,
    last_start: Option<u64>,
    last_end: Option<u64>,
}

async fn inner_main() -> Result<(), String> {
    const SERIAL: u64 = 0x4274E020EBB2D27E;
    let client = connect_insecure(51837).await.unwrap();
    let mut sub = client.stream_topic::<TraceReportTopic>(SERIAL).await.unwrap();
    let mut tasks = HashMap::<u32, TaskData>::new();
    let mut last = Instant::now();
    let mut bytes = 0;

    let mut exec_last_idle: Option<u64> = None;
    let mut exec_last_active: Option<u64> = None;
    let mut exec_ticks_idle = 0u64;
    let mut exec_ticks_active = 0u64;
    let mut last_delta = u64::MAX;
    // let mut extra = vec![];

    loop {
        let mut msg = sub.recv().await.unwrap();
        bytes += msg.events_cobs.len();

        // TODO: this doesn't handle wraparounds
        for ch in msg.events_cobs.split_inclusive_mut(|v| *v == 0) {
            if ch.is_empty() {
                println!("WRAP");
                continue;
            }
            // let ch = if extra.is_empty() {
            //     ch
            // } else {
            //     extra.extend_from_slice(ch);
            //     extra.as_mut_slice()
            // };

            let Ok(evt) = postcard::from_bytes_cobs::<Event>(ch) else {
                continue;
            };

            if last_delta == u64::MAX {
                if let Event::ExecutorPollStart { tick } = evt {
                    last_delta = tick;
                } else {
                    continue;
                }
            }

            // println!("{evt:?}");
            match evt {
                Event::TaskNew { .. } => {
                    // NOTE: uses abs time
                },
                Event::TaskExecBegin { tick, task_id } => {
                    // NOTE: uses delta time
                    let tick = tick + last_delta;
                    // Ready -> Active
                    let tsk = tasks
                        .entry(task_id)
                        .or_default();
                    tsk.last_start = Some(tick);
                    if let Some(t) = tsk.last_ready.take() {
                        let delta = tick.checked_sub(t).unwrap();
                        tsk.ticks_waiting += delta;
                    }
                },
                Event::TaskExecEnd { tick, task_id } => {
                    // NOTE: uses delta time
                    let tick = tick + last_delta;
                    // Active -> Idle
                    let tsk = tasks
                        .entry(task_id)
                        .or_default();
                    tsk.last_end = Some(tick);
                    if let Some(t) = tsk.last_start.take() {
                        let delta = tick.checked_sub(t).unwrap();
                        tsk.ticks_active += delta;
                    }
                },
                Event::TaskReadyBegin { tick, task_id } => {
                    // NOTE: uses delta time
                    let tick = tick + last_delta;
                    // Idle -> Ready
                    let tsk = tasks
                        .entry(task_id)
                        .or_default();
                    tsk.readies += 1;
                    tsk.last_ready = Some(tick);
                    if let Some(t) = tsk.last_end.take() {
                        let delta = tick.checked_sub(t).unwrap();
                        tsk.ticks_idle += delta;
                    }
                },
                Event::ExecutorIdle { tick  } => {
                    // NOTE: uses delta time
                    let tick = tick + last_delta;
                    if let Some(t) = exec_last_active.take() {
                        let delta = tick.checked_sub(t).unwrap();
                        exec_ticks_active += delta;
                    }
                    exec_last_idle = Some(tick);
                    // println!("exec_last_idle: {exec_last_idle:?}");
                },
                Event::ExecutorPollStart { tick  } => {
                    // NOTE: uses abs time, sets delta time
                    last_delta = tick;
                    if let Some(t) = exec_last_idle.take() {
                        let delta = tick.checked_sub(t).unwrap();
                        exec_ticks_idle += delta;
                    }
                    exec_last_active = Some(tick);
                    // println!("exec_last_active: {exec_last_active:?}");
                },
                Event::TaskIdentify { .. } => {},
            }
        }
        if last.elapsed() >= Duration::from_secs(1) {
            last = Instant::now();
            let mut vec = vec![];
            for (k, v) in tasks.iter_mut() {
                vec.push((*k, v.clone()));
                v.ticks_active = 0;
                v.ticks_waiting = 0;
                v.ticks_idle = 0;
            }
            vec.sort_by_key(|(k, _v)| *k);
            let ttl_exec_ticks = exec_ticks_idle + exec_ticks_active;
            let ttl_exec_ticks = ttl_exec_ticks as f64;

            println!();
            println!("KiB/s: {:.02}", bytes as f64 / 1024.0);
            println!(
                "EXECUTOR:   IDLE: {:03.02}%, ACTV: {:03.02}%, TICK: {}",
                100.0 * exec_ticks_idle as f64 / ttl_exec_ticks,
                100.0 * exec_ticks_active as f64 / ttl_exec_ticks,
                ttl_exec_ticks,
            );
            exec_ticks_idle = 0;
            exec_ticks_active = 0;
            bytes = 0;
            for (k, v) in vec {
                let ttl_ticks = v.ticks_idle + v.ticks_active + v.ticks_waiting;
                print!("T-{k:08X}: ");
                if ttl_ticks == 0 {
                    print!("IDLE: 100.0%, ");
                    print!("ACTV: 00.00%, ");
                    print!("WAIT: 00.00%, ");
                    print!("WAKE: n/a, ");
                    print!("TICK: n/a");
                } else {
                    let ttl_ticks = ttl_ticks as f64;
                    print!("IDLE: {:03.02}%, ", 100.0 * v.ticks_idle as f64 / ttl_ticks);
                    print!("ACTV: {:03.02}%, ", 100.0 * v.ticks_active as f64 / ttl_ticks);
                    print!("WAIT: {:03.02}%, ", 100.0 * v.ticks_waiting as f64 / ttl_ticks);
                    print!("WAKE: {}, ", v.readies);
                    print!("TICK: {}", ttl_ticks);
                }

                println!();
            }
        }

    }
}
