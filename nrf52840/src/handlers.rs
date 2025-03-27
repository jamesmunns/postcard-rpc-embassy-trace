use embassy_time::{Instant, Timer};
use postcard_rpc::{header::VarHeader, server::Sender};
use template_icd::{HaltClearError, HaltClearResponse, LedState, SleepEndpoint, SleepMillis, SleptMillis, StageCommand, StageError, StageResponse, StartError, StartResponse};

use crate::{app::{AppTx, Context, LoadState, TaskContext}, load::{worker, HALT, STAGE}};

/// This is an example of a BLOCKING handler.
pub fn unique_id(context: &mut Context, _header: VarHeader, _arg: ()) -> u64 {
    context.unique_id
}

/// Also a BLOCKING handler
pub fn set_led(context: &mut Context, _header: VarHeader, arg: LedState) {
    match arg {
        LedState::Off => context.led.set_high(),
        LedState::On => context.led.set_low(),
    }
}

pub fn get_led(context: &mut Context, _header: VarHeader, _arg: ()) -> LedState {
    match context.led.is_set_high() {
        true => LedState::Off,
        false => LedState::On,
    }
}

pub async fn stage_task(context: &mut Context, _header: VarHeader, arg: StageCommand) -> StageResponse {
    match context.load_state {
        LoadState::Idle => {
            // defmt::println!("Spawning!");
            match context.spawner.spawn(worker(arg)) {
                Ok(_) => {
                    context.load_state = LoadState::Pending(1);
                    Ok(())
                },
                Err(_) => {
                    // what?
                    Err(StageError::OutOfSpace)
                },
            }
        },
        LoadState::Pending(n) => {
            // defmt::println!("Spawning!");
            match context.spawner.spawn(worker(arg)) {
                Ok(_) => {
                    context.load_state = LoadState::Pending(n + 1);
                    Ok(())
                },
                Err(_) => {
                    Err(StageError::OutOfSpace)
                },
            }
        },
        LoadState::Running(_) => Err(StageError::AlreadyRunning),
    }
}

pub async fn start_tasks(context: &mut Context, _header: VarHeader, _arg: ()) -> StartResponse {
    match context.load_state {
        LoadState::Idle => Err(StartError::NoTasksStaged),
        LoadState::Pending(n) => {
            STAGE.wake_all();
            context.load_state = LoadState::Running(n);
            Ok(n)
        },
        LoadState::Running(_) => Err(StartError::AlreadyRunning),
    }
}

pub fn is_drs(_context: &mut Context, _header: VarHeader, _arg: ()) -> bool {
    cfg!(feature = "drs-scheduler")
}

pub async fn halt_tasks(context: &mut Context, _header: VarHeader, _arg: ()) -> HaltClearResponse {
    match context.load_state {
        LoadState::Idle => Err(HaltClearError::NotRunning),
        LoadState::Pending(n) | LoadState::Running(n) => {
            HALT.wake_all();
            context.load_state = LoadState::Idle;
            Ok(n)
        }
    }
}

/// This is a SPAWN handler
///
/// The pool size of three means we can have up to three of these requests "in flight"
/// at the same time. We will return an error if a fourth is requested at the same time
#[embassy_executor::task(pool_size = 3)]
pub async fn sleep_handler(_context: TaskContext, header: VarHeader, arg: SleepMillis, sender: Sender<AppTx>) {
    // We can send string logs, using the sender
    let _ = sender.log_str("Starting sleep...").await;
    let start = Instant::now();
    Timer::after_millis(arg.millis.into()).await;
    let _ = sender.log_str("Finished sleep").await;
    // Async handlers have to manually reply, as embassy doesn't support returning by value
    let _ = sender.reply::<SleepEndpoint>(header.seq_no, &SleptMillis { millis: start.elapsed().as_millis() as u16 }).await;
}
