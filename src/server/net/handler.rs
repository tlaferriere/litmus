// `mem::uninitialized` replaced with `mem::MaybeUninit`,
// can't upgrade yet
#![allow(deprecated)]

use pyo3::prelude::*;
use pyo3::exceptions::{PyRuntimeError, PyIOError};

use std::io;
use std::sync::Arc;
use std::mem::MaybeUninit;
use std::sync::atomic::Ordering::Relaxed;
use std::net::Shutdown::Both;
use std::sync::atomic::AtomicBool;
use std::io::{Write, Read};

use bytes::{BytesMut, BufMut};
use once_cell::sync::OnceCell;

use crate::listener::PyreClientAddrPair;
use crate::server::parser::H11Parser;


const MAX_BUFFER_SIZE: usize = 512 * 1024;

static LOOP_CREATE_TASK: OnceCell<PyObject> = OnceCell::new();
static LOOP_REMOVE_READER: OnceCell<PyObject> = OnceCell::new();
static LOOP_REMOVE_WRITER: OnceCell<PyObject> = OnceCell::new();


/// This sets up the net package's global state, this is absolutely required
/// to stop large amounts of python calls and clones, this MUST be setup before
/// any listeners can be created otherwise you risk UB.
pub fn setup(
    loop_create_task: PyObject,
    loop_remove_reader: PyObject,
    loop_remove_writer: PyObject,
) {
    LOOP_CREATE_TASK.get_or_init(|| loop_create_task);
    LOOP_REMOVE_READER.get_or_init(|| loop_remove_reader);
    LOOP_REMOVE_WRITER.get_or_init(|| loop_remove_writer);
}


/// The PyreClientHandler struct is what handles all the actual interactions
/// with the socket, this can be reused several times over and is designed to
/// handle concurrent pipelined requests, hopefully we can support http/2 as
/// well as http/1.1 once h11 works. :-)
#[pyclass]
pub struct PyreClientHandler {
    client_handle: PyreClientAddrPair,
    parser: H11Parser,

    // buffers
    writing_buffer: BytesMut,

    // Pre-Built callbacks
    resume_reading: MaybeUninit<Arc<PyObject>>,
    resume_writing: MaybeUninit<Arc<PyObject>>,

    // state
    reading: Arc<AtomicBool>,
    writing: Arc<AtomicBool>,
}

/// The implementations for all initialisation of objects and existing object
#[pymethods]
impl PyreClientHandler {
    /// Used to create a new handler object, generally this should only be
    /// created when absolutely needed.
    #[new]
    fn new(client: PyreClientAddrPair) -> PyResult<Self> {
        let test = LOOP_REMOVE_READER.get();
        if test.is_none() {
            return Err(PyRuntimeError::new_err(
                "Global state has not been setup, \
                did you forget to call pyre.setup()?"
            ))
        }

        let new_parse = H11Parser::new(MAX_BUFFER_SIZE);

        Ok(PyreClientHandler {
            client_handle: client,
            parser: new_parse,
            writing_buffer: BytesMut::with_capacity(MAX_BUFFER_SIZE),

            resume_reading: MaybeUninit::<Arc<PyObject>>::uninit(),
            resume_writing: MaybeUninit::<Arc<PyObject>>::uninit(),

            reading: Arc::new(AtomicBool::new(true)),
            writing: Arc::new(AtomicBool::new(false)),
        })
    }

    /// This should only be called when the object is first made, if this
    /// is called after being called once you will run into ub because it
    /// will not drop the value.
    fn init(&mut self, add_reader: PyObject, add_writer: PyObject) {
        let mut resume_ptr = self.resume_reading.as_mut_ptr();
        unsafe { resume_ptr.write(Arc::new(add_reader)) };

        let mut resume_ptr = self.resume_writing.as_mut_ptr();
        unsafe { resume_ptr.write(Arc::new(add_writer)) };
    }

    /// This is used when recycle the handler objects as the memory allocations
    /// are pretty expensive and we need some way of controlling the ram usage
    /// because theres a weird leak otherwise.
    fn new_client(&mut self, client: PyreClientAddrPair) {
        self.reset_state();
        self.client_handle = client;
    }

    /// Resets all state the handler might have as to not interfere
    /// with new client handles.
    fn reset_state(&mut self) {
        self.writing_buffer.clear();

        self.reading.store(true, Relaxed);
        self.writing.store(false, Relaxed);
    }
}

/// All callback events e.g. when `EventLoop.add_reader calls the callback.
#[pymethods]
impl PyreClientHandler {
    /// Called when the event loop detects that the
    /// socket is able to be read from without blocking.
    fn poll_read(&mut self) -> PyResult<()> {
        match self.parser.read(&mut self.client_handle.client) {
            Ok(_) => Ok(()),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                return Ok(())
            },
            Err(e) => {
                return Err(PyIOError::new_err(format!("{:?}", e)))
            }
        }


        loop {
            match self.parser.parse() {
                Ok(_) => {

                },

                Err(_) => {

                }
            };
        }


        Ok(())
    }

    /// Called when the event loop detects that the socket
    /// is able to be written to without blocking.
    fn poll_write(&mut self) -> PyResult<()> {


        Ok(())
    }
}

/// General utils for handling the sockets
impl PyreClientHandler {
    #[cfg(target_os = "windows")]
    fn fd(&self) -> u64 {
        self.listener.as_raw_socket()
    }

    #[cfg(target_os = "unix")]
    fn fd(&self) -> i32 {
        self.listener.as_raw_fd()
    }

    fn close_and_cleanup(&mut self) -> PyResult<()> {
        if self.reading.load(Relaxed) {
            let cb = unsafe { LOOP_REMOVE_READER.get_unchecked() };

            let _ = Python::with_gil(|py| -> PyResult<PyObject> {
                cb.call1(py, (self.fd()),)
            })?;
        }

        if self.writing.load(Relaxed) {
            let cb = unsafe { LOOP_REMOVE_WRITER.get_unchecked() };

            let _ = Python::with_gil(|py| -> PyResult<PyObject> {
                cb.call1(py, (self.fd()),)
            })?;
        }
        let _ = self.client_handle.client.shutdown(Both);
        Ok(())
    }
}
