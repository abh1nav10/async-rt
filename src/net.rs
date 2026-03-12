#![allow(dead_code)]

use crate::atomicwaker::Torque;
use crate::runtime::AVAILABLE_PARALLELISM;
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};

// TODO: Fix the following:
// The current problem with this approach is that we cannot have multiple runtimes in the same
// process as they all will end up sharing this PROVIDER and the operations will get corrupted!
// Futhermore, I am currently unwrapping on the initiliazation as it must happen exactly once when
// the reactor thread gets spawned. If the aforementioned scenario exists, the reactor thread of
// other runtimes will just panic due to unwrap on an Err. Also, I not able to run multiple
// tests currently as when I create a new runtime for each one of them, the reactor threads of
// those panic for the same reason! When run indpendently, all the tests pass!
pub(crate) static PROVIDER: OnceLock<GlobalProvider> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct GlobalProvider {
    pub(crate) registry: Arc<mio::Registry>,
    pub(crate) slab: Arc<Torque>,
}

impl GlobalProvider {
    pub(crate) fn give_registry(&self) -> Arc<mio::Registry> {
        Arc::clone(&self.registry)
    }

    pub(crate) fn give_slab(&self) -> Arc<Torque> {
        Arc::clone(&self.slab)
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

        let slab = provider.give_slab();

        // We must always register the new waker first before reading from the socket because
        // reading first and registering after encountering ` WouldBlock` presents in front of us
        // the possibility of lost wake-ups. For example, assume after getting `WouldBlock` out
        // thredad gets prempted and the reactor receives events, it will keep waking up the wrong
        // waker and because that new information wont be read from the socket, its edge triggered
        // nature will cause to not get woken up anymore and our task stalls forever!
        //
        // We subtract (AVAILABLE_PARALLELISM + 1) because we added it while registering!
        slab.update(
            self.token.0 - (AVAILABLE_PARALLELISM + 1),
            cx.waker().clone(),
        );

        match self.listener.accept() {
            Ok((mut stream, addr)) => {
                let registry = provider.give_registry();

                // We insert without waker because the stream that we receive when used for reading
                // or writing will get its waker registered!
                let token = slab.insert_without_waker();

                // We add (AVAILABLE_PARALLELISM + 1) because everything upto available parallelism is
                // reserved for the reactor and the worker threads registered with the registry!
                let token = mio::Token((AVAILABLE_PARALLELISM + 1) + token);

                // We have this tiny loop to register the stream with the Poll instance!
                loop {
                    match registry.register(
                        &mut stream,
                        token,
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
                let stream = TcpStream::new(stream, token);
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

        let slab = provider.give_slab();
        let registry = provider.give_registry();

        let token = slab.insert_without_waker();
        // We add (AVAILABLE_PARALLELISM + 1) because everything upto available parallelism is reserved
        // for the reactor and the worker threads registered with the registry!
        let token = mio::Token((AVAILABLE_PARALLELISM + 1) + token);

        match registry.register(&mut listener, token, mio::Interest::READABLE) {
            Ok(_) => {
                let listener = TcpListener { listener, token };

                Ok(listener)
            }
            Err(e) => {
                slab.remove(token.0 - (AVAILABLE_PARALLELISM + 1));
                Err(e)
            }
        }
    }

    pub fn accept<'a>(&'a self) -> AcceptFuture<'a> {
        AcceptFuture {
            listener: &self.listener,
            // TODO: Handle token number generation!
            token: self.token,
            stream: None,
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
    token: Option<mio::Token>,
}

impl Drop for StreamConnectFuture {
    fn drop(&mut self) {
        let slab = if let Some(provider) = PROVIDER.get() {
            provider.give_slab()
        } else {
            return;
        };

        // Mio deregisters the source on drop. So we only need to remove from the map!
        slab.remove(self.token.expect("Must be there").0 - (AVAILABLE_PARALLELISM + 1));
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

        let slab = provider.give_slab();

        if self.first_poll {
            // Insert the waker first before registering the stream!
            let token = slab.insert(cx.waker().clone());
            let token = mio::Token(token + (AVAILABLE_PARALLELISM + 1));

            self.token = Some(token);

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
            let token = self
                .token
                .expect("Has to be there as we put it in the first poll!");
            // Re register the waker before reading!
            slab.update(token.0 - (AVAILABLE_PARALLELISM + 1), cx.waker().clone());

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

        let fut = StreamConnectFuture {
            stream: Some(stream),
            first_poll: true,
            token: None,
        };
        Ok(fut)
    }
}
