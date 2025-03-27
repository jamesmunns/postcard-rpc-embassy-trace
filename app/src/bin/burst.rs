use std::{collections::HashMap, time::Duration};

use demo::System;
use postcard::accumulator::{CobsAccumulator, FeedResult};
use poststation_sdk::connect_insecure;
use template_icd::*;
use tokio::time::sleep;

use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<(), String> {
    println!("------------------------------------------------------------");
    for load in 0..20 {
        let cfg = BurstConfig {
            load_tasks: load,
            load_deadline_base: 15 * 32768,
            load_deadline_incr: 30,
            load_deadline_work: 65,
            timing_pulse_interval: 32768 / 4,
            crit_start_delay: 32768 * 2,
            crit_work_ticks: 33,
            crit_sleep_ticks: 11,
            crit_deadline_ticks: ((33 * 3) + (11 * 2)) * 3,
            crit_iters: 3,
        };
        println!("CFG:");
        println!("{cfg:#?}");
        println!();
        let mut ctxt = inner_main(cfg).await.unwrap();
        println!("Total Events: {}", ctxt.all_events.len());
        ctxt.system.render();
        println!("------------------------------------------------------------");
    }

    Ok(())
}

const SERIAL: u64 = 0x4274E020EBB2D27E;

#[derive(Debug)]
struct BurstConfig {
    // Load tasks fire 16x. All load tasks will wait for the timing pulse,
    // then perform their work, then
    load_tasks: usize,
    load_deadline_base: u64,
    load_deadline_incr: u64,
    load_deadline_work: u32,

    // Timing task will fire 16x. It sleeps first, then fires the timing
    // pulse.
    timing_pulse_interval: u64,

    // Critical task will fire once. It will be idle for the start delay,
    // then start on the next timing pulse. The deadline timer will start
    // after the first timing pulse. It will then follow the pattern
    // (work, sleep), iter times, and will skip the last sleep. So if
    // iter == 3, it'll do work, sleep, work, sleep, work.
    crit_start_delay: u64,
    crit_work_ticks: u64,
    crit_sleep_ticks: u64,
    crit_deadline_ticks: u64,
    crit_iters: usize,
}

async fn inner_main(cfg: BurstConfig) -> Result<Context, String> {
    let client = connect_insecure(51837).await.unwrap();
    sleep(Duration::from_secs(1)).await;

    println!("Halting...");
    let _ = client
        .proxy_endpoint::<HaltTasksEndpoint>(SERIAL, 200, &())
        .await
        .unwrap();
    sleep(Duration::from_millis(1000)).await;
    println!("Halted.");

    let is_drs = client
        .proxy_endpoint::<IsDrsEndpoint>(SERIAL, 201, &())
        .await
        .unwrap();
    println!("Is DRS?: {is_drs}");

    let mut tasks = vec![];

    // ten load tasks
    for i in 0..cfg.load_tasks {
        let mut steps = heapless::Vec::<Step, 32>::new();
        for _ in 0..16 {
            steps.push(Step::WaitTrigger).unwrap();
            // two ms of work
            steps.push(Step::WorkTicks { ticks: cfg.load_deadline_work }).unwrap();
        }
        tasks.push(StageCommand {
            ident: i as u32,
            steps,
            loops: false,
            deadline_ticks: cfg.load_deadline_base + (cfg.load_deadline_incr * i as u64),
            loop_delay_ticks: 0,
            start_delay_ticks: 0,
            manual_start_stop: false,
        });
    }

    // One "critical" task, starts after two seconds
    tasks.push(StageCommand {
        ident: 100,
        steps: {
            let mut steps = heapless::Vec::<Step, 32>::new();
            steps.push(Step::WaitTrigger).unwrap();

            // Start the deadline
            steps.push(Step::ManualDeadlineStart).unwrap();

            for _ in 0..cfg.crit_iters {
                steps.push(Step::WorkTicks { ticks: cfg.crit_work_ticks as u32 }).unwrap();
                steps.push(Step::SleepTicks { ticks: cfg.crit_sleep_ticks as u32 }).unwrap();
            }
            // pop the last sleep
            steps.pop().unwrap();

            // Stop the deadline
            steps.push(Step::ManualDeadlineStop).unwrap();

            steps
        },
        loops: false,
        deadline_ticks: cfg.crit_deadline_ticks,
        loop_delay_ticks: 0,
        start_delay_ticks: cfg.crit_start_delay,
        manual_start_stop: true,
    });

    // One "timing" task, fires timing pulses
    tasks.push(StageCommand {
        ident: 110,
        steps: {
            let mut steps = heapless::Vec::<Step, 32>::new();

            // Fire pulses 16x
            for _ in 0..16 {
                steps.push(Step::SleepTicks { ticks: cfg.timing_pulse_interval as u32 }).unwrap();
                steps.push(Step::WakeTrigger).unwrap();
            }

            steps
        },
        loops: false,
        deadline_ticks: 17 * cfg.timing_pulse_interval,
        loop_delay_ticks: 0,
        start_delay_ticks: 0,
        manual_start_stop: false,
    });



    let mut sub = client
        .stream_topic::<TraceReportTopic>(SERIAL)
        .await
        .unwrap();


    tokio::task::spawn({
        let client = client.clone();
        async move {
            println!("Sending tasks...");
            for task in tasks.iter() {
                client
                    .proxy_endpoint::<StageTaskEndpoint>(SERIAL, task.ident, task)
                    .await
                    .unwrap()
                    .unwrap();
                sleep(Duration::from_millis(100)).await;
            }
            println!("Starting in 3s:");
            sleep(Duration::from_secs(3)).await;
            client
                .proxy_endpoint::<StartTasksEndpoint>(SERIAL, 200, &())
                .await
                .unwrap()
                .unwrap();
        }
    });

    let mut acc = CobsAccumulator::<64>::new();
    println!("Running...");
    let mut context = Context::default();
    loop {
        let msg = sub.recv().await.unwrap();

        let mut window = msg.events_cobs.as_slice();
        'cobs: while !window.is_empty() {
            window = match acc.feed_ref::<Event>(window) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(_new_wind) => todo!(),
                FeedResult::DeserError(new_wind) => {
                    println!("WARN: Decode Error");
                    new_wind
                }
                FeedResult::Success {
                    data: evt,
                    remaining,
                } => {
                    handle_evt(&evt, &mut context);
                    remaining
                }
            };
        }

        if !context.worker_tasks.is_empty() && context.worker_tasks.iter().all(|(_k, v)| {
            v.start.is_some() && v.stop.is_some()
        }) {
            break;
        }
    }

    let mut out = context.worker_tasks.iter().collect::<Vec<_>>();
    out.sort_unstable_by_key(|(k, _v)| *k);
    for (_k, v) in out {
        let start = v.start.unwrap();
        let stop = v.stop.unwrap();
        let deadline = v.deadline.unwrap();
        println!("{}, dur: {}, deadline: {}, pass?: {}",
            v.name,
            stop - start,
            deadline,
            (stop - start) <= deadline,
        );
    }
    Ok(context)
}

#[derive(Debug)]
struct BurstTask {
    name: String,
    start: Option<u64>,
    deadline: Option<u64>,
    stop: Option<u64>,
}

#[derive(Debug, Default)]
struct Context {
    cur_tick: Option<u64>,
    worker_tasks: HashMap<u32, BurstTask>,
    all_events: Vec<Event>,
    system: System,
}

fn handle_evt(evt: &Event, context: &mut Context) {
    let cur_tick = *if let Some(ct) = context.cur_tick.as_mut() {
        ct
    } else if let Event::ExecutorPollStart { tick } = evt {
        context.cur_tick.insert(*tick)
    } else {
        return;
    };

    context.all_events.push(evt.clone());
    context.system.handle_evt(evt.clone(), 0);

    match evt {
        // Event::TaskNew { tick, task_id } => todo!(),
        // Event::TaskExecBegin { tick, task_id } => todo!(),
        // Event::TaskExecEnd { tick, task_id } => todo!(),
        // Event::TaskReadyBegin { tick, task_id } => todo!(),
        // Event::ExecutorIdle { tick } => todo!(),
        Event::ExecutorPollStart { tick } => {
            context.cur_tick = Some(*tick);
        },
        Event::TaskIdentify {
            tick,
            task_id,
            name,
        } => {
            let _tick = cur_tick + *tick;
            if name.starts_with("WRK") {
                context
                    .worker_tasks
                    .entry(*task_id)
                    .or_insert_with(|| BurstTask {
                        name: name.to_string(),
                        start: None,
                        deadline: None,
                        stop: None,
                     });
            }
        }
        Event::DeadlineStart {
            tick,
            task_id,
            deadline,
        } => {
            let tick = cur_tick + *tick;
            let tsk = context.worker_tasks.get_mut(task_id).unwrap();
            assert!(tsk.start.is_none());
            assert!(tsk.deadline.is_none());

            tsk.start = Some(tick);
            tsk.deadline = Some(*deadline);
            tsk.stop = None;
        }
        Event::DeadlineStop { tick, task_id } => {
            let tick = cur_tick + *tick;
            let tsk = context.worker_tasks.get_mut(task_id).unwrap();
            assert!(tsk.stop.is_none());
            tsk.stop = Some(tick);
        }
        _ => {}
    }
}
