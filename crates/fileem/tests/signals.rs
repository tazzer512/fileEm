//! `fileem::signals` (run without the test harness: it sends this process signals, which every
//! thread must have blocked, so nothing else may run here).

fn main() {
    #[cfg(unix)]
    a_termination_signal_reaches_the_handler_once_even_when_sent_early();
}

#[cfg(unix)]
fn a_termination_signal_reaches_the_handler_once_even_when_sent_early() {
    use std::sync::mpsc;
    use std::time::Duration;
    let blocked = fileem::signals::block().expect("block");
    // SAFETY: sent to ourselves; the only thread has SIGTERM blocked, so it stays pending
    unsafe { libc::kill(libc::getpid(), libc::SIGTERM) };
    let (tx, rx) = mpsc::channel();
    blocked.wait_in_background(Duration::from_secs(60), move || tx.send(()).unwrap()).expect("wait");
    rx.recv_timeout(Duration::from_secs(5)).expect("the handler ran for the pending signal");
    assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "once");
    println!("signals: ok");
}
