use std::{collections::HashMap, time::{Duration, Instant}};

use poststation_sdk::{connect, connect_insecure, PoststationClient};
use template_icd::{Event, HaltTasksEndpoint, StageCommand, StageTaskEndpoint, StartTasksEndpoint, Step, TraceReportTopic};
use tokio::time::sleep;

use tikv_jemallocator::Jemalloc;
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() -> Result<(), String> {
    inner_main().await
}

// #[derive(Default, Clone, Debug)]
// struct TaskData {
//     readies: usize,
//     ticks_active: u64,
//     ticks_waiting: u64,
//     ticks_idle: u64,
//     last_ready: Option<u64>,
//     last_start: Option<u64>,
//     last_end: Option<u64>,
// }
const SERIAL: u64 = 0x4274E020EBB2D27E;

async fn inner_main() -> Result<(), String> {
    let client = connect_insecure(51837).await.unwrap();
    tokio::task::spawn(dump(client.clone()));
    sleep(Duration::from_secs(1)).await;

    println!("Halting...");
    let _ = client.proxy_endpoint::<HaltTasksEndpoint>(
        SERIAL,
        200,
        &()
    ).await.unwrap();
    sleep(Duration::from_millis(1000)).await;


    for i in 100..140 {
        let mut steps = heapless::Vec::new();
        let _ = steps.push(Step::SleepMs { ms: 4 * i });
        let _ = steps.push(Step::WorkMs { ms: (i - 99) / 4 });

        client.proxy_endpoint::<StageTaskEndpoint>(
            SERIAL,
            i,
            &StageCommand {
                ident: i,
                steps,
                loops: true,
                deadline_ticks: 400,
            }
        ).await.unwrap().unwrap();
    }

    println!("Halted, Starting in 1s:");
    sleep(Duration::from_millis(1000)).await;
    client.proxy_endpoint::<StartTasksEndpoint>(
        SERIAL,
        200,
        &()
    ).await.unwrap().unwrap();

    loop {
        sleep(Duration::from_millis(1000)).await;
    }
}

async fn dump(client: PoststationClient) {
    let mut sub = client.stream_topic::<TraceReportTopic>(SERIAL).await.unwrap();
    // let mut tasks = HashMap::<u32, TaskData>::new();
    let mut last = Instant::now();
    // let start = Instant::now();
    let mut bytes = 0;

    // let mut exec_last_idle: Option<u64> = None;
    // let mut exec_last_active: Option<u64> = None;
    // let mut exec_ticks_idle = 0u64;
    // let mut exec_ticks_active = 0u64;
    // let mut extra = vec![];
    let mut ct = 0;

    loop {
        let mut msg = sub.recv().await.unwrap();
        bytes += msg.events_cobs.len();

        // TODO: this doesn't handle wraparounds
        for ch in msg.events_cobs.split_inclusive_mut(|v| *v == 0) {
        //     if ch.is_empty() {
        //         continue;
        //     }
        //     let ch = if extra.is_empty() {
        //         ch
        //     } else {
        //         extra.extend_from_slice(ch);
        //         extra.as_mut_slice()
        //     };

            let Ok(_evt) = postcard::from_bytes_cobs::<Event>(ch) else {
        //         extra = ch.to_vec();
                continue;
            };
            ct += 1;
            if last.elapsed() > Duration::from_secs(1) {
                last = Instant::now();
                println!("{ct}");
                ct = 0;
            }
        //     extra.clear();
        //     // println!("{evt:?}");
        }
    }
}
