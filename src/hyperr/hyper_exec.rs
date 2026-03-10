use hyper::rt::Executor;
use std::future::Future;
use std::sync::Arc;

#[non_exhaustive]
pub struct HyperExecutor(Arc<crate::runtime::RuntimeInstance>);

impl HyperExecutor {
    pub fn new(executor: Arc<crate::runtime::RuntimeInstance>) -> Self {
        HyperExecutor(executor)
    }
}

// Clone implementation is required to pass in the builder API of Hyper!
impl Clone for HyperExecutor {
    fn clone(&self) -> Self {
        HyperExecutor::new(Arc::clone(&self.0))
    }
}

impl<F> Executor<F> for HyperExecutor
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, future: F) {
        self.0.spawn(future);
    }
}
