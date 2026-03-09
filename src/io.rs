use crate::TcpStream;
use crate::net::PROVIDER;
use futures_util::{AsyncRead, AsyncWrite};
use std::io::{Error, ErrorKind};
use std::io::{Read, Write};
use std::net::Shutdown;
use std::pin::Pin;
use std::task::{Context, Poll};

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, Error>> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let map = provider.give_map();

        let mut guard = map.lock().expect("Got back a poinsoned lock!");

        // We need to register the waker if `will_wake returns false` because if `poll_read`
        // returns Poll::Ready, the underlying future created by the method on the extension traits
        // like AsyncReadExt might get dropped and they might create a new one on the next call and
        // if we do not update the waker, our call will hang!
        let entry = guard
            .entry(self.token)
            .or_insert_with(|| cx.waker().clone());

        if !entry.will_wake(cx.waker()) {
            *entry = cx.waker().clone();
        }

        drop(guard);

        match self.stream.read(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            // Handling errors as per the documentation in the futures_util crate!
            Err(e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::other(e))),
        }
    }
}

impl AsyncWrite for TcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let map = provider.give_map();

        let mut guard = map.lock().expect("Got back a poinsoned lock!");

        let entry = guard
            .entry(self.token)
            .or_insert_with(|| cx.waker().clone());

        if !entry.will_wake(cx.waker()) {
            *entry = cx.waker().clone();
        }

        drop(guard);

        match self.stream.write(buf) {
            Ok(n) => Poll::Ready(Ok(n)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::other(e))),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    // We attempt to shutdown use `mio::TcpStream::shutdown`. If other tasks are also using the same
    // stream, they will not hang as the OS will notify all registered wakers which in all cases in
    // this runtime is just one that the socket is being closed and the reactor will wake the
    // appropriate wakers up!
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let map = provider.give_map();

        let mut guard = map.lock().expect("Got back a poinsoned lock!");

        // In this method, this check is required only on the first Poll because as we return
        // Poll::Ready the future will be dropped and we wont Poll again! We anyway keep it as the
        // following check will fail and we won't be cloning the waker!
        let entry = guard
            .entry(self.token)
            .or_insert_with(|| cx.waker().clone());

        if !entry.will_wake(cx.waker()) {
            *entry = cx.waker().clone();
        }

        drop(guard);

        // We try to shutdown the Write half of the connection!
        match self.stream.shutdown(Shutdown::Write) {
            Ok(_) => Poll::Ready(Ok(())),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
            Err(e) if e.kind() == ErrorKind::Interrupted => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::other(e))),
        }
    }
}
