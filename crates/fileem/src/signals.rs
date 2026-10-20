//! Saving on SIGTERM, SIGHUP and SIGINT (logging out, `kill`, Ctrl+C in a terminal), like the
//! Java app's shutdown hook. The main thread blocks them before any other thread starts (threads
//! inherit the mask), and one thread waits for them with `sigwait`, so the handler is ordinary
//! code, not a signal handler. It runs once; if the app hasn't quit `grace` later (a hung UI
//! thread), or a second signal arrives, the process exits anyway. Elsewhere this does nothing
//! (the 2 s autosave still covers most of it).

use std::time::Duration;

/// The termination signals, blocked in the threads started from now on.
pub struct Blocked {
    #[cfg(unix)]
    set: libc::sigset_t,
}

/// Blocks the termination signals in this thread and the ones it starts later. Call it first
/// thing in `main`.
pub fn block() -> std::io::Result<Blocked> {
    #[cfg(unix)]
    {
        // SAFETY: plain libc calls on a local signal set
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            for s in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
                libc::sigaddset(&mut set, s);
            }
            let r = libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            if r != 0 {
                return Err(std::io::Error::from_raw_os_error(r));
            }
            Ok(Blocked { set })
        }
    }
    #[cfg(not(unix))]
    Ok(Blocked {})
}

impl Blocked {
    /// Calls `handler` on a thread of its own when a termination signal arrives (also one sent
    /// before this call), then exits the process if it is still running `grace` later.
    pub fn wait_in_background(self, grace: Duration, handler: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            let set = self.set;
            std::thread::Builder::new().name("fileem-signals".into()).spawn(move || {
                let mut sig = 0;
                // SAFETY: waits on the set blocked above
                while unsafe { libc::sigwait(&set, &mut sig) } != 0 {}
                handler();
                std::thread::Builder::new()
                    .name("fileem-exit".into())
                    .spawn(move || {
                        std::thread::sleep(grace);
                        std::process::exit(128 + sig);
                    })
                    .ok();
                // a second signal: no more waiting
                let mut again = 0;
                while unsafe { libc::sigwait(&set, &mut again) } != 0 {}
                std::process::exit(128 + again);
            })?;
        }
        #[cfg(not(unix))]
        let _ = (grace, handler);
        Ok(())
    }
}
