//! Contains support for user-managed thread pools, represented by the
//! the [`ThreadPool`] type (see that struct for details).
//!
//! [`ThreadPool`]: struct.ThreadPool.html

use core::fmt;
#[cfg(feature = "thread-pinning")]
use core_affinity::CoreId;
use crate::{crossbeam_workstealing_pool::{ThreadPool, ThreadPoolWorker}, parker::Parker};
#[cfg(feature = "numa-aware")]
use crate::numa_utils::ProcessingUnitDistance;
use private::{private_decl, private_impl};
use std::io;

mod private {
    //! The public parts of this private module are used to create traits
    //! that cannot be implemented outside of our own crate.  This way we
    //! can feel free to extend those traits without worrying about it
    //! being a breaking change for other implementations.
    
    /// If this type is pub but not publicly reachable, third parties
    /// can't name it and can't implement traits using it.
    pub struct PrivateMarker;
    
    macro_rules! private_decl {
        () => {
            /// This trait is private; this method exists to make it
            /// impossible to implement outside the crate.
            #[doc(hidden)]
            fn __executors_private__(&self) -> $crate::builder::private::PrivateMarker;
        };
    }
    
    macro_rules! private_impl {
        () => {
            fn __executors_private__(&self) -> $crate::builder::private::PrivateMarker {
                $crate::builder::private::PrivateMarker
            }
        };
    }

    pub(super) use {private_decl, private_impl};
}

/// Thread builder used for customization via
/// [`ThreadPoolBuilder::spawn_handler`](struct.ThreadPoolBuilder.html#method.spawn_handler).
pub struct ThreadBuilder<P: Parker + Clone + 'static> {
    pub(crate) name: Option<String>,
    pub(crate) stack_size: Option<usize>,
    pub(crate) worker: ThreadPoolWorker<P>,
}

impl<P: Parker + Clone + 'static> ThreadBuilder<P> {
    /// Gets the index of this thread in the pool, within `0..num_threads`.
    pub fn index(&self) -> usize {
        *self.worker.id()
    }

    /// Gets the string that was specified by `ThreadPoolBuilder::name()`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Gets the value that was specified by `ThreadPoolBuilder::stack_size()`.
    pub fn stack_size(&self) -> Option<usize> {
        self.stack_size
    }

    /// Executes the main loop for this thread. This will not return until the
    /// thread pool is dropped.
    pub fn run(mut self) where P: Parker + Clone + 'static {
        self.worker.run()
    }
}

impl<P: Parker + Clone + 'static + fmt::Debug> fmt::Debug for ThreadBuilder<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ThreadBuilder")
            .field("pool", &self.worker)
            .field("name", &self.name)
            .field("stack_size", &self.stack_size)
            .finish()
    }
}

/// Generalized trait for spawning a thread in the `Registry`.
///
/// This trait is pub-in-private -- E0445 forces us to make it public,
/// but we don't actually want to expose these details in the API.
pub trait ThreadSpawn<P> {
    private_decl! {}

    /// Spawn a thread with the `ThreadBuilder` parameters, and then
    /// call `ThreadBuilder::run()`.
    fn spawn(&self, thread: ThreadBuilder<P>) -> io::Result<()>
        where P: Parker + Clone + 'static;
}

/// Spawns a thread in the "normal" way with `std::thread::Builder`.
///
/// This type is pub-in-private -- E0445 forces us to make it public,
/// but we don't actually want to expose these details in the API.
#[derive(Debug, Default)]
pub struct DefaultSpawn;

impl<P> ThreadSpawn<P> for DefaultSpawn {
    private_impl! {}

    fn spawn(&self, thread: ThreadBuilder<P>) -> io::Result<()>
        where P: Parker + Clone + 'static,
    {
        let mut b = std::thread::Builder::new();
        if let Some(name) = thread.name() {
            b = b.name(name.to_owned());
        }
        if let Some(stack_size) = thread.stack_size() {
            b = b.stack_size(stack_size);
        }
        b.spawn(|| thread.run())?;
        Ok(())
    }
}

/// Spawns a thread with a user's custom callback.
///
/// This type is pub-in-private -- E0445 forces us to make it public,
/// but we don't actually want to expose these details in the API.
#[derive(Debug)]
pub struct CustomSpawn<F>(F);

impl<F> CustomSpawn<F>
{
    pub(super) fn new(spawn: F) -> Self {
        CustomSpawn(spawn)
    }
}

impl<P: Parker + Clone + 'static, F> ThreadSpawn<P> for CustomSpawn<F>
where
    F: Fn(ThreadBuilder<P>) -> io::Result<()>,
{
    private_impl! {}

    #[inline]
    fn spawn(&self, thread: ThreadBuilder<P>) -> io::Result<()>
    {
        (self.0)(thread)
    }
}

/// Used to create a new [`ThreadPool`].
pub struct ThreadPoolBuilder<S: ?Sized = DefaultSpawn> {
    /* /// The number of threads in the rayon thread pool.
    num_threads: usize, */

    /* /// Custom closure, if any, to handle a panic that we cannot propagate
    /// anywhere else.
    panic_handler: Option<Box<PanicHandler>>, */

    /// Closure to compute the name of a thread.
    get_thread_name: Option<Box<dyn Fn(usize) -> String + Send + Sync>>,

    /// The stack size for the created worker threads
    stack_size: Option<usize>,

    /// Closure invoked on worker thread start.
    start_handler: Option<Box<StartHandler>>,

    /* /// Closure invoked on worker thread exit.
    exit_handler: Option<Box<ExitHandler>>, */

    /// Closure invoked to spawn threads.
    spawn_handler: S,
}

/* /// The type for a panic handling closure. Note that this same closure
/// may be invoked multiple times in parallel.
type PanicHandler = dyn Fn(Box<dyn Any + Send>) + Send + Sync; */

/// The type for a closure that gets invoked when a thread starts. The
/// closure is passed the index of the thread on which it is invoked.
/// Note that this same closure may be invoked multiple times in parallel.
type StartHandler = dyn Fn(usize) + Send + Sync;

/* /// The type for a closure that gets invoked when a thread exits. The
/// closure is passed the index of the thread on which is is invoked.
/// Note that this same closure may be invoked multiple times in parallel.
type ExitHandler = dyn Fn(usize) + Send + Sync; */

// NB: We can't `#[derive(Default)]` because `S` is left ambiguous.
impl Default for ThreadPoolBuilder {
    fn default() -> Self {
        ThreadPoolBuilder {
            /* num_threads: 0,
            panic_handler: None, */
            get_thread_name: None,
            stack_size: None,
            start_handler: None,
            /* exit_handler: None, */
            spawn_handler: DefaultSpawn,
        }
    }
}

impl ThreadPoolBuilder {
    /// Creates and returns a valid rayon thread pool builder, but does not initialize it.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Note: the `S: ThreadSpawn` constraint is an internal implementation detail for the
/// default spawn and those set by [`spawn_handler`](#method.spawn_handler).
impl<S: Send + Sync> ThreadPoolBuilder<S>
{
    /// Creates a new thread pool capable of executing `threads` number of jobs concurrently.
    ///
    /// Must supply a `parker` that can handle the requested number of threads.
    ///
    /// # Panics
    ///
    /// - This function will panic if `threads` is 0.
    /// - It will also panic if `threads` is larger than the `ThreadData::MAX_THREADS` value of the provided `parker`.
    ///
    /// # Core Affinity
    ///
    /// If compiled with `thread-pinning` it will assign a worker to each cores,
    /// until it runs out of cores or workers. If there are more workers than cores,
    /// the extra workers will be "floating", i.e. not pinned.
    ///
    /// # Examples
    ///
    /// Create a new thread pool capable of executing four jobs concurrently:
    ///
    /// ```
    /// use executors::*;
    /// use executors::builder;
    /// use executors::crossbeam_workstealing_pool::ThreadPool;
    ///
    /// let pool = builder::ThreadPoolBuilder::new().build(4, parker::small());
    /// # pool.shutdown();
    /// ```
    pub fn build<P: Parker + Clone + 'static>(self, threads: usize, parker: P) -> ThreadPool<P>
        where S: ThreadSpawn<P> + 'static,
    {
        ThreadPool::new(self, threads, parker)
    }

    /// Creates a new thread pool capable of executing `threads` number of jobs concurrently with a particular core affinity.
    ///
    /// For each core id in the `core` slice, it will generate a single thread pinned to that id.
    /// Additionally, it will create `floating` number of unpinned threads.
    ///
    /// # Panics
    ///
    /// - This function will panic if `cores.len() + floating` is 0.
    /// - This function will panic if `cores.len() + floating` is greater than `parker.max_threads()`.
    /// - This function will panic if no core ids can be accessed.
    #[cfg(feature = "thread-pinning")]
    pub fn build_with_affinity<P: Parker + Clone + 'static>(self, cores: &[CoreId], floating: usize, parker: P) -> ThreadPool<P>
        where S: ThreadSpawn<P> + 'static,
    {
        ThreadPool::with_affinity(self, cores, floating, parker)
    }

    /// Creates a new thread pool capable of executing `threads` number of jobs concurrently with a particular core affinity.
    ///
    /// For each core id in the `core` slice, it will generate a single thread pinned to that id.
    /// Additionally, it will create `floating` number of unpinned threads.
    ///
    /// Internally the stealers will use the provided PU distance matrix to prioritise stealing from
    /// queues that are "closer" by, in order to try and reduce memory movement across caches and NUMA nodes.
    ///
    /// # Panics
    ///
    /// - This function will panic if `cores.len() + floating` is 0.
    /// - This function will panic if `cores.len() + floating` is greater than `parker.max_threads()`.
    /// - This function will panic if no core ids can be accessed.
    #[cfg(feature = "numa-aware")]
    pub fn build_with_numa_affinity<P: Parker + Clone + 'static>(
        self,
        cores: &[CoreId],
        floating: usize,
        parker: P,
        pu_distance: ProcessingUnitDistance,
    ) -> ThreadPool<P>
        where S: ThreadSpawn<P> + 'static,
    {
        ThreadPool::with_numa_affinity(self, cores, floating, parker, pu_distance)
    }
}

impl<S: ?Sized> ThreadPoolBuilder<S> {
    /// Sets a custom function for spawning threads.
    ///
    /// Note that the threads will not exit until after the pool is dropped. It
    /// is up to the caller to wait for thread termination if that is important
    /// for any invariants.
    pub fn spawn_handler<P, F>(self, spawn: F) -> ThreadPoolBuilder<CustomSpawn<F>>
    where
        S: Send + Sized,
        P: Parker + Clone + 'static,
        F: Fn(ThreadBuilder<P>) -> io::Result<()>,
    {
        ThreadPoolBuilder {
            spawn_handler: CustomSpawn::new(spawn),
            /* num_threads: self.num_threads,
            panic_handler: self.panic_handler, */
            get_thread_name: self.get_thread_name,
            stack_size: self.stack_size,
            start_handler: self.start_handler,
            /* exit_handler: self.exit_handler, */
        }
    }

    /// Returns a reference to the current spawn handler.
    pub(crate) fn get_spawn_handler(&self) -> &S {
        &self.spawn_handler
    }

    /* /// Get the number of threads that will be used for the thread
    /// pool. See `num_threads()` for more information.
    pub(crate) fn get_num_threads(&self) -> usize {
        if self.num_threads > 0 {
            self.num_threads
        } else {
            match env::var("RAYON_NUM_THREADS")
                .ok()
                .and_then(|s| usize::from_str(&s).ok())
            {
                Some(x) if x > 0 => return x,
                Some(x) if x == 0 => return num_cpus::get(),
                _ => {}
            }

            // Support for deprecated `RAYON_RS_NUM_CPUS`.
            match env::var("RAYON_RS_NUM_CPUS")
                .ok()
                .and_then(|s| usize::from_str(&s).ok())
            {
                Some(x) if x > 0 => x,
                _ => num_cpus::get(),
            }
        }
    } */

    /// Get the thread name for the thread with the given index.
    pub(crate) fn get_thread_name(&self, index: usize) -> Option<String> {
        let f = self.get_thread_name.as_ref()?;
        Some(f(index))
    }

    /// Sets a closure which takes a thread index and returns
    /// the thread's name.
    pub fn thread_name<F>(mut self, closure: F) -> Self
    where
        S: Sized,
        F: Fn(usize) -> String + Send + Sync + 'static,
    {
        self.get_thread_name = Some(Box::new(closure));
        self
    }

    /* pub fn num_threads(mut self, num_threads: usize) -> Self {
        self.num_threads = num_threads;
        self
    }

    /// Returns a copy of the current panic handler.
    fn take_panic_handler(&mut self) -> Option<Box<PanicHandler>> {
        self.panic_handler.take()
    }

    pub fn panic_handler<H>(mut self, panic_handler: H) -> Self
    where
        H: Fn(Box<dyn Any + Send>) + Send + Sync + 'static,
    {
        self.panic_handler = Some(Box::new(panic_handler));
        self
    } */

    /// Get the stack size of the worker threads
    pub(crate) fn get_stack_size(&self) -> Option<usize> {
        self.stack_size
    }

    /// Sets the stack size of the worker threads
    pub fn stack_size(mut self, stack_size: usize) -> Self
    where
        S: Sized,
    {
        self.stack_size = Some(stack_size);
        self
    }

    /// Takes the current thread start callback, leaving `None`.
    pub(crate) fn get_start_handler(&self) -> Option<&StartHandler> {
        self.start_handler.as_deref()
    }

    /// Sets a callback to be invoked on thread start.
    ///
    /// The closure is passed the index of the thread on which it is invoked.
    /// Note that this same closure may be invoked multiple times in parallel.
    /// If this closure panics, the panic will be passed to the panic handler.
    /// If that handler returns, then startup will continue normally.
    pub fn start_handler<H>(mut self, start_handler: H) -> Self
    where
        H: Fn(usize) + Send + Sync + 'static,
        S: Sized,
    {
        self.start_handler = Some(Box::new(start_handler));
        self
    }

    /* /// Returns a current thread exit callback, leaving `None`.
    fn take_exit_handler(&mut self) -> Option<Box<ExitHandler>> {
        self.exit_handler.take()
    }

    /// Sets a callback to be invoked on thread exit.
    ///
    /// The closure is passed the index of the thread on which it is invoked.
    /// Note that this same closure may be invoked multiple times in parallel.
    /// If this closure panics, the panic will be passed to the panic handler.
    /// If that handler returns, then the thread will exit normally.
    pub fn exit_handler<H>(mut self, exit_handler: H) -> Self
    where
        H: Fn(usize) + Send + Sync + 'static,
    {
        self.exit_handler = Some(Box::new(exit_handler));
        self
    } */
}
