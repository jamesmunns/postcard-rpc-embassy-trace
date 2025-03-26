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
        let _ = steps.push(Step::SleepUs { us: 2 * i * 1000 });
        let _ = steps.push(Step::WorkUs { us: 1000 * (i - 99) / 8 });
        let _ = steps.push(Step::Yield);
        let _ = steps.push(Step::WorkUs { us: 1000 * (i - 99) / 8 });
        let _ = steps.push(Step::SleepUs { us: 2 * i * 1000 });
        let _ = steps.push(Step::WorkUs { us: 1000 * (i - 99) / 8 });

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

    println!("Running...");
    loop {
        sleep(Duration::from_millis(1000)).await;
    }
}

