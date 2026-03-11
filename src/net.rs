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
    pub(crate) map: Arc<Mutex<HashMap<mio::Token, Option<Waker>>>>,
}

impl GlobalProvider {
    pub(crate) fn give_registry(&self) -> Arc<mio::Registry> {
        Arc::clone(&self.registry)
    }

    pub(crate) fn give_map(&self) -> Arc<Mutex<HashMap<mio::Token, Option<Waker>>>> {
        Arc::clone(&self.map)
    }
}

pub struct TcpListener {
    listener: mio::net::TcpListener,
    token: mio::Token,
}

pub struct AcceptFuture<'a> {
    listener: &'a mio::net::TcpListener,
    token: mio::Token,
    stream: Option<mio::net::TcpStream>,
    stream_token: mio::Token,
}

impl Future for AcceptFuture<'_> {
    type Output = Result<(TcpStream, std::net::SocketAddr), Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let map = provider.give_map();

        // We must always register the new waker first before reading from the socket because
        // reading first and registering after encountering ` WouldBlock` presents in front of us
        // the possibility of lost wake-ups. For example, assume after getting `WouldBlock` out
        // thredad gets prempted and the reactor receives events, it will keep waking up the wrong
        // waker and because that new information wont be read from the socket, its edge triggered
        // nature will cause to not get woken up anymore and our task stalls forever!
        let mut guard = map.lock().expect("Got back a poisoned Mutex!");

        let value = guard
            .get_mut(&self.token)
            .expect("We put the token while binding!");

        let new_waker = cx.waker();

        if let Some(waker) = value.as_ref() {
            if !waker.will_wake(new_waker) {
                *value = Some(new_waker.clone());
            }
        } else {
            *value = Some(new_waker.clone());
        }

        // Drop the guard before proceeding further!
        drop(guard);

        match self.listener.accept() {
            Ok((mut stream, addr)) => {
                let registry = provider.give_registry();

                // We have this tiny loop to register the stream with the Poll instance!
                loop {
                    // TODO: Fix this and fix the waker wakeup bugs!
                    match registry.register(
                        &mut stream,
                        self.stream_token,
                        mio::Interest::READABLE | mio::Interest::WRITABLE,
                    ) {
                        Ok(_) => break,
                        Err(e) if e.kind() == ErrorKind::Interrupted => {
                            continue;
                        }
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }

                // Return the stream with the `stream_token`
                let stream = TcpStream::new(stream, self.stream_token);
                Poll::Ready(Ok((stream, addr)))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                // Since we have already updated the waker, we will be rightly woken up again!

                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(Error::new(e.kind(), e))),
        }
    }
}

impl TcpListener {
    pub fn bind(addr: std::net::SocketAddr) -> Result<TcpListener, Error> {
        let provider = PROVIDER
            .get()
            .ok_or(Error::other("Could not find PROVIDER, try again!"))?;

        let mut listener = mio::net::TcpListener::bind(addr)?;

        let map = provider.give_map();
        let registry = provider.give_registry();

        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let token = mio::Token(token);

        map.lock()
            .expect("Got back a poisoned lock!")
            .insert(token, None);

        match registry.register(&mut listener, token, mio::Interest::READABLE) {
            Ok(_) => {
                let listener = TcpListener { listener, token };

                Ok(listener)
            }
            Err(e) => {
                map.lock()
                    .expect("Got back a poisened lock!")
                    .remove(&token);
                Err(e)
            }
        }
    }

    pub fn accept<'a>(&'a self) -> AcceptFuture<'a> {
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);

        AcceptFuture {
            listener: &self.listener,
            // TODO: Handle token number generation!
            token: self.token,
            stream: None,
            stream_token: mio::Token(token),
        }
    }
}

// TODO: Impl Drop!!
// Must store the underlying mio::net::TcpStream and our registered token as we need them while
// implementing traits like futures_util::AsyncRead etc.
pub struct TcpStream {
    pub(crate) stream: mio::net::TcpStream,
    pub(crate) token: mio::Token,
}

pub struct StreamConnectFuture {
    // We use an `Option` only to use `Option::take` to take the stream out when returning `Poll::Ready`
    stream: Option<mio::net::TcpStream>,
    first_poll: bool,
    token: mio::Token,
}

impl Drop for StreamConnectFuture {
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

impl Future for StreamConnectFuture {
    type Output = Result<(TcpStream, core::net::SocketAddr), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let provider = if let Some(provider) = PROVIDER.get() {
            provider
        } else {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        };

        let token = self.token;

        let map = provider.give_map();

        let mut guard = map.lock().expect("Got back a poisoned Mutex!");

        // Re register the waker before reading!
        let waker = cx.waker();
        let entry = guard.entry(token).or_insert_with(|| Some(waker.clone()));

        if !entry.as_ref().expect("Has to be there").will_wake(waker) {
            *entry = Some(waker.clone());
        }

        drop(guard);

        if self.first_poll {
            let stream = self.stream.as_mut().expect("Has to be there");

            let registry = provider.give_registry();

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
                                let stream = TcpStream::new(stream, token);
                                Poll::Ready(Ok((stream, addr)))
                            }

                            // According to the documentation, we must check for ErrorKind::NotConnected or
                            // libc::EINPROGRESS which also maps to ErrorKind::NotConnected. peer_addr
                            // cannot return WOULDBLOCK as it is not an IO operation. The peer address is
                            // either present or not present. That's it!
                            Err(e) if e.kind() == ErrorKind::NotConnected => {
                                // We have already updated the waker, so we will not loose wakeups
                                // here!

                                Poll::Pending
                            }

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
    fn new(stream: mio::net::TcpStream, token: mio::Token) -> Self {
        Self { stream, token }
    }

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
