//! This crate provides a wrapper type of Tokio's JoinHandle: `ScopedTask`, which aborts the task when it's dropped.
//! `ScopedTask` can still be awaited to join the child-task, and abort-on-drop will still trigger while it is being awaited.
//!
//! For example, if task A spawned task B but is doing something else, and task B is waiting for task C to join,
//! aborting A will also abort both B and C.

use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct ScopedTask<T> {
    inner: JoinHandle<T>,
}

impl<T> Drop for ScopedTask<T> {
    fn drop(&mut self) {
        self.inner.abort()
    }
}

impl<T> Future for ScopedTask<T> {
    type Output = <JoinHandle<T> as Future>::Output;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx)
    }
}

impl<T> From<JoinHandle<T>> for ScopedTask<T> {
    fn from(inner: JoinHandle<T>) -> Self {
        Self { inner }
    }
}

impl<T> Deref for ScopedTask<T> {
    type Target = JoinHandle<T>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
