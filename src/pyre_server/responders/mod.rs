use std::sync::Arc;
use pyo3::PyObject;
use crossbeam::queue::SegQueue;

pub mod sender;
pub mod receiver;

/// The payload that gets sent to the receiver half of the channel.
pub type Payload = (bool, Vec<u8>);

/// The queue of Python waiters to be woken up on a given event.
pub(crate) type WakerQueue = Arc<SegQueue<PyObject>>;