use core::pin::pin;

use embassy_futures::{select::{select, Either}, yield_now};
use embassy_time::{Instant, Timer};
use template_icd::{StageCommand, Step};
use maitake_sync::WaitQueue;

use crate::trace::task_identify;

#[cfg(feature = "drs-scheduler")]
use embassy_executor::raw::Deadline;


pub static STAGE: WaitQueue = WaitQueue::new();
pub static HALT: WaitQueue = WaitQueue::new();

#[embassy_executor::task(pool_size = 50)]
pub async fn worker(cmd: StageCommand) {
    {
        use core::fmt::Write;
        let mut name = heapless::String::<10>::new();
        let _ = name.push_str("WRK-");
        let _ = write!(&mut name, "{:06X}", cmd.ident);
        task_identify(name.as_str()).await;
    }
    let halt_fut = HALT.wait();
    let mut halt_fut = pin!(halt_fut);
    let _ = halt_fut.as_mut().subscribe();

    let stage_fut = STAGE.wait();
    let mut stage_fut = pin!(stage_fut);
    let _ = stage_fut.as_mut().subscribe();

    let run_fut = async {
        // Wait for the trigger
        let _ = stage_fut.await;

        #[cfg(feature = "drs-scheduler")]
        Deadline::set_current_task_deadline_after(cmd.deadline_ticks).await;

        loop {
            if cmd.steps.is_empty() && cmd.loops {
                panic!();
            }
            for step in cmd.steps.iter() {
                match step {
                    Step::SleepUs { us } => Timer::after_micros((*us).into()).await,
                    Step::WorkUs { us } => {
                        let now = Instant::now();
                        let ttl = u64::from(*us);
                        while now.elapsed().as_micros() < ttl {
                            // busy!
                        }
                    },
                    Step::Yield => {
                        yield_now().await;
                    }
                }
            }

            if !cmd.loops {
                break;
            } else {
                #[cfg(feature = "drs-scheduler")]
                Deadline::set_current_task_deadline_after(cmd.deadline_ticks).await;
            }
        }
    };

    match select(run_fut, halt_fut).await {
        Either::First(_) => defmt::info!("Worker task {=u32} completed", cmd.ident),
        Either::Second(_) => defmt::info!("Worker task {=u32} halted", cmd.ident),
    }
}
