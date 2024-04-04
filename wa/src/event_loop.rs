use crate::native::apple::frameworks::{CFRunLoopSourceCreate,
CFRunLoopSourceContext,
CFRunLoopGetMain,
CFRunLoopSourceRef, CFRelease, CFRunLoopSourceSignal, CFIndex, CFRunLoopAddSource, kCFRunLoopCommonModes, CFRunLoopWakeUp};
use std::ptr;
use std::os::raw::c_void;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::atomic::AtomicBool;
use std::fmt;

static EVENT_LOOP_CREATED: AtomicBool = AtomicBool::new(false);

pub struct EventLoop<T: 'static> {
    // Event sender and receiver, used for EventLoopProxy.
    pub sender: mpsc::Sender<T>,
    pub receiver: Rc<mpsc::Receiver<T>>,
}

#[derive(Debug)]
pub enum EventLoopError {
    /// The event loop can't be re-created.
    RecreationAttempt,
    /// Application has exit with an error status.
    ExitFailure(i32),
}

impl<T> EventLoop<T> {
    /// Creates an [`EventLoopProxy`] that can be used to dispatch user events
    /// to the main event loop, possibly from another thread.
    pub fn create_proxy(&self) -> EventLoopProxy<T> {
        EventLoopProxy::new(self.sender.clone())
    }

    pub fn build() -> Result<EventLoop<T>, EventLoopError> {
        if EVENT_LOOP_CREATED.swap(true, Ordering::Relaxed) {
            return Err(EventLoopError::RecreationAttempt);
        }

        use std::sync::mpsc::channel;
        let (tx, rx) = channel();

        Ok(EventLoop {
            sender: tx,
            receiver: rx.into(),
        })
    }
}

/// Used to send custom events to [`EventLoop`].
// pub struct EventLoopProxy<T: 'static> {
//     sender: mpsc::Sender<T>,
// }

// impl<T: 'static> EventLoopProxy<T> {
//     /// Send an event to the [`EventLoop`] from which this proxy was created. This emits a
//     /// `UserEvent(event)` event in the event loop, where `event` is the value passed to this
//     /// function.
//     ///
//     /// Returns an `Err` if the associated [`EventLoop`] no longer exists.
//     ///
//     /// [`UserEvent(event)`]: Event::UserEvent
//     pub fn send_event(&self, event: T) -> Result<(), EventLoopClosed<T>> {
//         self.event_loop_proxy.send_event(event)
//     }
// }

impl<T: 'static> fmt::Debug for EventLoopProxy<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("EventLoopProxy { .. }")
    }
}

pub struct EventLoopProxy<T> {
    sender: mpsc::Sender<T>,
    source: CFRunLoopSourceRef,
}

unsafe impl<T: Send> Send for EventLoopProxy<T> {}
unsafe impl<T: Send> Sync for EventLoopProxy<T> {}

impl<T> Drop for EventLoopProxy<T> {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.source as _);
        }
    }
}

impl<T> Clone for EventLoopProxy<T> {
    fn clone(&self) -> Self {
        EventLoopProxy::new(self.sender.clone())
    }
}

impl<T> EventLoopProxy<T> {
    fn new(sender: mpsc::Sender<T>) -> Self {
        unsafe {
            // just wake up the eventloop
            extern "C" fn event_loop_proxy_handler(_: *const c_void) {}

            // adding a Source to the main CFRunLoop lets us wake it up and
            // process user events through the normal OS EventLoop mechanisms.
            let rl = CFRunLoopGetMain();
            let mut context = CFRunLoopSourceContext {
                version: 0,
                info: ptr::null_mut(),
                retain: None,
                release: None,
                copyDescription: None,
                equal: None,
                hash: None,
                schedule: None,
                cancel: None,
                perform: event_loop_proxy_handler,
            };
            let source =
                CFRunLoopSourceCreate(ptr::null_mut(), CFIndex::max_value() - 1, &mut context);
            CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
            CFRunLoopWakeUp(rl);

            EventLoopProxy { sender, source }
        }
    }

    pub fn send_event(&self, event: T) -> Result<(), EventLoopClosed<T>> {
        self.sender
            .send(event)
            .map_err(|mpsc::SendError(x)| EventLoopClosed(x))?;
        unsafe {
            // let the main thread know there's a new event
            CFRunLoopSourceSignal(self.source);
            let rl = CFRunLoopGetMain();
            CFRunLoopWakeUp(rl);
        }
        Ok(())
    }
}

/// The error that is returned when an [`EventLoopProxy`] attempts to wake up an [`EventLoop`] that
/// no longer exists.
///
/// Contains the original event given to [`EventLoopProxy::send_event`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventLoopClosed<T>(pub T);

impl<T> fmt::Display for EventLoopClosed<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tried to wake up a closed `EventLoop`")
    }
}

impl<T: fmt::Debug> std::error::Error for EventLoopClosed<T> {}