use std::{
    fs,
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

// How long each step of stopping a process group may take
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    // Time the processes get to exit after SIGTERM before SIGKILL is sent
    pub term_grace: Duration,
    // Time after SIGKILL in which the group must be gone, or stopping failed
    pub verify_timeout: Duration,
    // How often the group is looked at
    pub poll: Duration,
    // SIGKILL is sent again this often while something is left
    pub kill_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            term_grace: Duration::from_millis(100),
            verify_timeout: Duration::from_secs(3),
            poll: Duration::from_millis(10),
            kill_retry: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Term,
    Kill,
}

// What stopping a group needs from the system, so the escalation can be tested without
// real processes
pub trait GroupOps {
    fn signal(&mut self, signal: Signal);

    // The processes of the group that are still running (zombies are not running)
    fn members(&mut self) -> Vec<u32>;
}

pub struct SystemGroup {
    pgid: u32,
}

impl SystemGroup {
    pub fn new(pgid: u32) -> Self {
        Self { pgid }
    }
}

impl GroupOps for SystemGroup {
    fn signal(&mut self, signal: Signal) {
        let name = match signal {
            Signal::Term => "-TERM",
            Signal::Kill => "-KILL",
        };
        let group = format!("-{}", self.pgid);

        // A group that is already gone makes kill fail, which is the goal
        let _ = Command::new("kill")
            .args([name, "--", group.as_str()])
            .status();
    }

    fn members(&mut self) -> Vec<u32> {
        group_members(self.pgid)
    }
}

// Every running process whose process group is `pgid`, read from /proc. A zombie is a
// process that already ended and only waits to be collected, so it is not counted.
pub fn group_members(pgid: u32) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;

            // "pid (name) state ppid pgrp ...", the name may contain spaces and
            // parentheses, so the fields are read after the last closing parenthesis
            let rest = stat.rsplit_once(')')?.1;
            let mut fields = rest.split_whitespace();

            let state = fields.next()?;
            let _parent = fields.next()?;
            let group: u32 = fields.next()?.parse().ok()?;

            (group == pgid && !matches!(state, "Z" | "X")).then_some(pid)
        })
        .collect()
}

// Sends SIGKILL at once and does not wait, used to make requests in flight fail quickly. The
// group is still stopped and checked afterwards.
pub fn kill_now(pgid: u32) {
    SystemGroup::new(pgid).signal(Signal::Kill);
}

// SIGTERM, a short grace, then SIGKILL until nothing of the group is left. Ok only when the
// group has really gone, so a signal that did not work is never taken for success.
pub fn terminate_verified(pgid: u32, timing: &Timing) -> Result<(), String> {
    terminate_with(&mut SystemGroup::new(pgid), timing)
}

pub fn terminate_with(group: &mut dyn GroupOps, timing: &Timing) -> Result<(), String> {
    group.signal(Signal::Term);

    if wait_until_empty(group, timing.term_grace, timing.poll) {
        return Ok(());
    }

    group.signal(Signal::Kill);

    let deadline = Instant::now() + timing.verify_timeout;
    let mut last_kill = Instant::now();

    loop {
        let left = group.members();

        if left.is_empty() {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "{} process(es) still running after SIGKILL: {:?}",
                left.len(),
                left
            ));
        }

        if last_kill.elapsed() >= timing.kill_retry {
            group.signal(Signal::Kill);
            last_kill = Instant::now();
        }

        thread::sleep(timing.poll);
    }
}

fn wait_until_empty(group: &mut dyn GroupOps, duration: Duration, poll: Duration) -> bool {
    let deadline = Instant::now() + duration;

    loop {
        if group.members().is_empty() {
            return true;
        }

        if Instant::now() >= deadline {
            return false;
        }

        thread::sleep(poll);
    }
}

// Stops the process group a child leads and collects the child. The child must have been
// started in its own group (setsid), so its pid is the group id. The child is only collected
// once the group is gone, waiting for a process that cannot be killed could block forever.
pub fn stop_child_group(child: &mut Child) -> Result<(), String> {
    #[cfg(unix)]
    {
        let pgid = child.id();

        let result = terminate_verified(pgid, &Timing::default())
            .map_err(|message| format!("process group {pgid}: {message}"));

        if result.is_ok() {
            let _ = child.wait();
        }

        result
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();

        Ok(())
    }
}
