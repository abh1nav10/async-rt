#![allow(dead_code)]

use crate::runtime::AVAILABLE_PARALLELISM;
use std::collections::HashMap;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

// TODO: Fix the following:
// The current problem with this approach is that we cannot have multiple runtimes in the same
// process as they all will end up sharing this PROVIDER and the operations will get corrupted!
// Futhermore, I am currently unwrapping on the initiliazation as it must happen exactly once when
// the reactor thread gets spawned. If the aforementioned scenario exists, the reactor thread of
// other runtimes will just panic due to unwrap on an Err. Also, I not able to run multiple
// tests currently as when I create a new runtime for each one of them, the reactor threads of
// those panic for the same reason! When run indpendently, all the tests pass!
pub(crate) static PROVIDER: OnceLock<GlobalProvider> = OnceLock::new();

// Currently using this as the source of unique tokens! I will be using the slab data structure in
// the future once I study it properly which will then ensure token uniqueness at minimal cost as
// the insertion operation will itself return the index at which the token was inserted which we
// will then add to AVAILABLE_PARALLELISM to get the token number that we will use for our token
static NEXT_TOKEN: AtomicUsize = AtomicUsize::new(AVAILABLE_PARALLELISM + 1);

#[derive(Debug)]
pub(crate) struct GlobalProvider {
    pub(crate) registry: Arc<mio::Registry>,
    pub(crate) map: Arc<Mutex<HashMap<mio::Token, Waker>>>,
}

impl GlobalProvider {
    pub(crate) fn give_registry(&self) -> Arc<mio::Registry> {
        Arc::clone(&self.registry)
    }

    pub(crate) fn give_map(&self) -> Arc<Mutex<HashMap<mio::Token, Waker>>> {
        Arc::clone(&self.map)
    }
}

pub struct TcpListener;

pub struct AcceptFuture {
    listener: mio::net::TcpListener,
    first_poll: bool,
    token: mio::Token,
}

impl Drop for AcceptFuture {
    fn drop(&mut self) {
        let map = if let Some(provider) = PROVIDER.get() {
            provider.give_map()
        } else {
            return;
        };

        // Mio deregisters the source on drop. So we only need to remove from the map!
        // TODO: Get rid of unwrap.
        map.lock().unwrap().remove(&self.token);
    }
}

impl Future for AcceptFuture {
    type Output = Result<(mio::net::TcpStream, std::net::SocketAddr), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let token = self.token;

        if self.first_poll {
            let registry = provider.give_registry();

            let map = provider.give_map();

            // We must first put the entry in the map before registering in order to prevent the
            // possibility of the reactor being woken up by the OS, it seeing no waker for the
            // underlying token due to us entering it into the map after registering the event with
            // the Poll and ending up doing nothing. That would lead to the future never completing
            // since the wakeup has already been lost.
            map.lock().unwrap().insert(token, cx.waker().clone());

            match registry.register(
                &mut self.listener,
                token,
                mio::Interest::READABLE | mio::Interest::WRITABLE,
            ) {
                Ok(_) => {
                    self.first_poll = false;

                    Poll::Pending
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Err(e) => Poll::Ready(Err(Error::new(e.kind(), e))),
            }
        } else {
            match self.listener.accept() {
                Ok(stream) => Poll::Ready(Ok(stream)),
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    // We do not need to reregister the waker as it points to the same task;
                    // map.lock().unwrap().insert(token, cx.waker().clone());

                    Poll::Pending
                }
                Err(e) => Poll::Ready(Err(Error::new(e.kind(), e))),
            }
        }
    }
}

impl TcpListener {
    pub fn accept(addr: std::net::SocketAddr) -> Result<AcceptFuture, Error> {
        let listener = mio::net::TcpListener::bind(addr)?;
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);

        let fut = AcceptFuture {
            listener,
            first_poll: true,
            // TODO: Handle token number generation!
            token: mio::Token(token),
        };
        Ok(fut)
    }
}

pub struct TcpStream;

pub struct StreamConnectFuture {
    // We use an `Option` only to use `Option::take` to take the stream out when returning `Poll::Ready`
    stream: Option<mio::net::TcpStream>,
    first_poll: bool,
    token: mio::Token,
}

impl Future for StreamConnectFuture {
    type Output = Result<(mio::net::TcpStream, core::net::SocketAddr), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let token = self.token;

        if self.first_poll {
            let stream = self.stream.as_mut().expect("Has to be there");

            let registry = provider.give_registry();
            let map = provider.give_map();

            map.lock().unwrap().insert(token, cx.waker().clone());

            match registry.register(
                stream,
                token,
                mio::Interest::READABLE | mio::Interest::WRITABLE,
            ) {
                Ok(_) => {
                    self.first_poll = false;

                    Poll::Pending
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                Err(e) => Poll::Ready(Err(Error::new(e.kind(), e))),
            }
        } else {
            let stream = self.stream.as_ref().expect("Has to be there!");
            let take_error = stream.take_error();

            // Following the documentation in `mio::net::TcpStream::connect`!
            match take_error {
                Ok(n) => {
                    if n.is_none() {
                        // The documentation asks us to check the peer_addr method!
                        match stream.peer_addr() {
                            Ok(addr) => {
                                let stream = self.stream.take().expect("Has to be there!");
                                Poll::Ready(Ok((stream, addr)))
                            }

                            // According to the documentation, we must check for ErrorKind::NotConnected or
                            // libc::EINPROGRESS which also maps to ErrorKind::NotConnected. peer_addr
                            // cannot return WOULDBLOCK as it is not an IO operation. The peer address is
                            // either present or not present. That's it!
                            Err(e) if e.kind() == ErrorKind::NotConnected => Poll::Pending,

                            Err(e) => Poll::Ready(Err(Error::other(e))),
                        }
                    } else {
                        Poll::Ready(Err(Error::other("Something went wrong!")))
                    }
                }
                Err(e) => Poll::Ready(Err(Error::other(e))),
            }
        }
    }
}

impl TcpStream {
    pub fn connect(addr: std::net::SocketAddr) -> Result<StreamConnectFuture, Error> {
        let stream = mio::net::TcpStream::connect(addr)?;
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);

        let fut = StreamConnectFuture {
            stream: Some(stream),
            first_poll: true,
            token: mio::Token(token),
        };
        Ok(fut)
    }
}
