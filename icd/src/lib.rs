#![cfg_attr(not(feature = "use-std"), no_std)]

use postcard_rpc::{endpoints, topics, TopicDirection};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct SleepMillis {
    pub millis: u16,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct SleptMillis {
    pub millis: u16,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub enum LedState {
    Off,
    On,
}

#[cfg(not(feature = "use-std"))]
#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct Report<'a> {
    pub events_cobs: &'a [u8],
}

#[cfg(feature = "use-std")]
#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct Report {
    pub events_cobs: Vec<u8>,
}

// These aren't actually sent directly as we need to
// batch them
#[cfg(not(feature = "use-std"))]
#[derive(Debug, Serialize, Deserialize, Schema, Clone)]
pub enum Event<'a> {
    // _embassy_trace_task_new
    TaskNew {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_task_exec_begin
    TaskExecBegin {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_task_exec_end
    TaskExecEnd {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_task_ready_begin
    TaskReadyBegin {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_executor_idle
    ExecutorIdle {
        tick: u64,
    },
    // _embassy_trace_poll_start
    ExecutorPollStart {
        tick: u64,
    },
    // task_identify
    TaskIdentify {
        tick: u64,
        task_id: u32,
        name: &'a str,
    },
    DeadlineStart {
        tick: u64,
        task_id: u32,
        deadline: u64,
    },
    DeadlineStop {
        tick: u64,
        task_id: u32,
    }
}

#[cfg(feature = "use-std")]
#[derive(Debug, Serialize, Deserialize, Schema, Clone)]
pub enum Event {
    // _embassy_trace_task_new
    TaskNew {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_task_exec_begin
    TaskExecBegin {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_task_exec_end
    TaskExecEnd {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_task_ready_begin
    TaskReadyBegin {
        tick: u64,
        task_id: u32,
    },
    // _embassy_trace_executor_idle
    ExecutorIdle {
        tick: u64,
    },
    // _embassy_trace_poll_start
    ExecutorPollStart {
        tick: u64,
    },
    // task_identify
    TaskIdentify {
        tick: u64,
        task_id: u32,
        name: String,
    },
    DeadlineStart {
        tick: u64,
        task_id: u32,
        deadline: u64,
    },
    DeadlineStop {
        tick: u64,
        task_id: u32,
    }
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub enum Step {
    SleepTicks {
        ticks: u32,
    },
    WorkTicks {
        ticks: u32,
    },
    Yield,
    WaitTrigger,
    WakeTrigger,
    ManualDeadlineStart,
    ManualDeadlineStop,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub struct StageCommand {
    pub ident: u32,
    pub steps: heapless::Vec<Step, 32>,
    pub loops: bool,
    // If loops == false, this is a single deadline
    // If loops == true, this is re-set after every completion of `steps`
    pub deadline_ticks: u64,
    pub loop_delay_ticks: u64,
    pub start_delay_ticks: u64,
    pub manual_start_stop: bool,
}

#[derive(Debug, Serialize, Deserialize, Schema)]
pub enum StageError {
    OutOfSpace,
    AlreadyRunning,
}

pub type StageResponse = Result<(), StageError>;

#[derive(Debug, Serialize, Deserialize, Schema)]
pub enum StartError {
    NoTasksStaged,
    AlreadyRunning,
}

pub type StartResponse = Result<u32, StartError>;

#[derive(Debug, Serialize, Deserialize, Schema)]
pub enum HaltClearError {
    NotRunning,
}

pub type HaltClearResponse = Result<u32, HaltClearError>;

// ---

// Endpoints spoken by our device
//
// GetUniqueIdEndpoint is mandatory, the others are examples
endpoints! {
    list = ENDPOINT_LIST;
    | EndpointTy                | RequestTy     | ResponseTy            | Path                          |
    | ----------                | ---------     | ----------            | ----                          |
    | GetUniqueIdEndpoint       | ()            | u64                   | "poststation/unique_id/get"   |
    | RebootToPicoBoot          | ()            | ()                    | "template/picoboot/reset"     |
    | SleepEndpoint             | SleepMillis   | SleptMillis           | "template/sleep"              |
    | SetLedEndpoint            | LedState      | ()                    | "template/led/set"            |
    | GetLedEndpoint            | ()            | LedState              | "template/led/get"            |
    | StageTaskEndpoint         | StageCommand  | StageResponse         | "task/stage"                  |
    | StartTasksEndpoint        | ()            | StartResponse         | "task/start"                  |
    | HaltTasksEndpoint         | ()            | HaltClearResponse     | "task/haltclear"              |
    | IsDrsEndpoint             | ()            | bool                  | "drs/is_drs"                  |
}

// incoming topics handled by our device
topics! {
    list = TOPICS_IN_LIST;
    direction = TopicDirection::ToServer;
    | TopicTy                   | MessageTy     | Path              |
    | -------                   | ---------     | ----              |
}

// outgoing topics handled by our device
topics! {
    list = TOPICS_OUT_LIST;
    direction = TopicDirection::ToClient;
    | TopicTy                   | MessageTy     | Path                  | Cfg                           |
    | -------                   | ---------     | ----                  | ---                           |
    | TraceReportTopic          | Report<'a>    | "trace/report"        | cfg(not(feature = "use-std")) |
    | TraceReportTopic          | Report        | "trace/report"        | cfg(feature = "use-std")      |
    | FakeEventTopic            | Event<'a>     | "trace/fake/event"    | cfg(not(feature = "use-std")) |
    | FakeEventTopic            | Event         | "trace/fake/event"    | cfg(feature = "use-std")      |
}
