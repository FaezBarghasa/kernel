#![forbid(unsafe_code)]

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use crossbeam_queue::ArrayQueue;
use spin::Mutex;
use zerocopy::{FromBytes, IntoBytes, Immutable};

use crate::sched::scx_types::{ScxOperation, ScxPolicyInfo, ScxRequest, ScxResponse, ScxTaskData};

/// Watchdog statistics for SCX performance.
#[derive(Debug, Clone, Copy, Default, FromBytes, IntoBytes, Immutable)]
#[repr(C)]
pub struct ScxStats {
    pub total_requests: u64,
    pub total_responses: u64,
    pub timeouts: u64,
    pub fallbacks_to_eevdf: u64,
    pub avg_response_time_ns: u64,
}

/// Bridge between kernel scheduler and user-space SCX policy.
pub struct ScxBridge {
    /// Queue for requests from kernel to user-space
    request_queue: Arc<ArrayQueue<ScxRequest>>,

    /// Queue for responses from user-space to kernel
    response_queue: Arc<ArrayQueue<ScxResponse>>,

    /// Currently registered SCX policy
    active_policy: Mutex<Option<ScxPolicyInfo>>,

    /// Atomic counter for request IDs
    next_request_id: AtomicU64,

    /// Flag indicating if SCX is enabled
    enabled: AtomicBool,

    /// Timeout for SCX responses (nanoseconds)
    timeout_ns: AtomicU64,

    /// Statistics
    stats: Mutex<ScxStats>,

    /// Unclaimed responses popped from the queue by other threads
    unclaimed_responses: Mutex<Vec<ScxResponse>>,

    /// Pending requests currently awaiting response (id, start_time_ns)
    pending_requests: Mutex<Vec<(u64, u64)>>,
}

impl ScxBridge {
    /// Creates a new SCX bridge with specified queue capacity.
    pub fn new(queue_capacity: usize) -> Self {
        Self {
            request_queue: Arc::new(ArrayQueue::new(queue_capacity)),
            response_queue: Arc::new(ArrayQueue::new(queue_capacity)),
            active_policy: Mutex::new(None),
            next_request_id: AtomicU64::new(1),
            enabled: AtomicBool::new(false),
            timeout_ns: AtomicU64::new(50_000), // Default timeout 50 microseconds (50_000 ns)
            stats: Mutex::new(ScxStats::default()),
            unclaimed_responses: Mutex::new(Vec::new()),
            pending_requests: Mutex::new(Vec::new()),
        }
    }

    /// Registers a user-space SCX policy.
    ///
    /// # Arguments
    /// * `policy_info` - Information about the policy to register
    ///
    /// # Returns
    /// * `Ok(())` if registration succeeded
    /// * `Err(ScxError)` if registration failed
    pub fn register_policy(&self, policy_info: ScxPolicyInfo) -> Result<(), ScxError> {
        if policy_info.name.is_empty() {
            return Err(ScxError::InvalidPolicyConfig("Policy name cannot be empty".into()));
        }
        if policy_info.timeout_ns == 0 {
            return Err(ScxError::InvalidPolicyConfig("Timeout must be greater than zero".into()));
        }

        let mut guard = self.active_policy.lock();
        if guard.is_some() {
            return Err(ScxError::PolicyAlreadyRegistered);
        }

        self.timeout_ns.store(policy_info.timeout_ns, Ordering::Release);
        *guard = Some(policy_info);
        self.enabled.store(true, Ordering::Release);
        Ok(())
    }

    /// Unregisters the current SCX policy.
    pub fn unregister_policy(&self) {
        let mut guard = self.active_policy.lock();
        *guard = None;
        self.enabled.store(false, Ordering::Release);

        // Drain queues
        while self.request_queue.pop().is_some() {}
        while self.response_queue.pop().is_some() {}
        self.unclaimed_responses.lock().clear();
        self.pending_requests.lock().clear();
    }

    /// Sends a request to the user-space SCX policy.
    ///
    /// # Arguments
    /// * `operation` - The scheduling operation to perform
    /// * `task_data` - Task data for the operation
    ///
    /// # Returns
    /// * `Some(ScxResponse)` if response received within timeout
    /// * `None` if timeout occurred (caller should fall back to EEVDF)
    pub fn send_request(
        &self,
        operation: ScxOperation,
        task_data: ScxTaskData,
    ) -> Option<ScxResponse> {
        if !self.is_enabled() {
            return None;
        }

        let request_id = self.next_request_id.fetch_add(1, Ordering::SeqCst);
        let start_time = crate::time::monotonic() as u64;

        let request = ScxRequest {
            request_id,
            operation,
            timestamp_ns: start_time,
            task_data,
        };

        // Add to pending tracking before pushing to queue
        self.pending_requests.lock().push((request_id, start_time));

        if self.request_queue.push(request).is_err() {
            // Queue full
            self.pending_requests.lock().retain(|(id, _)| *id != request_id);
            let mut stats = self.stats.lock();
            stats.fallbacks_to_eevdf += 1;
            return None;
        }

        let timeout = self.timeout_ns.load(Ordering::Acquire);

        loop {
            // 1. Check if our response is in the unclaimed list
            {
                let mut unclaimed = self.unclaimed_responses.lock();
                if let Some(pos) = unclaimed.iter().position(|r| r.request_id == request_id) {
                    let resp = unclaimed.remove(pos);
                    self.pending_requests.lock().retain(|(id, _)| *id != request_id);

                    // Update statistics
                    let mut stats = self.stats.lock();
                    stats.total_requests += 1;
                    stats.total_responses += 1;
                    let elapsed = (crate::time::monotonic() as u64).saturating_sub(start_time);
                    stats.avg_response_time_ns = (stats.avg_response_time_ns * 9 + elapsed) / 10;

                    return Some(resp);
                }
            }

            // 2. Try to pop a response
            if let Some(resp) = self.response_queue.pop() {
                if resp.request_id == request_id {
                    self.pending_requests.lock().retain(|(id, _)| *id != request_id);

                    let mut stats = self.stats.lock();
                    stats.total_requests += 1;
                    stats.total_responses += 1;
                    let elapsed = (crate::time::monotonic() as u64).saturating_sub(start_time);
                    stats.avg_response_time_ns = (stats.avg_response_time_ns * 9 + elapsed) / 10;

                    return Some(resp);
                } else {
                    // Stash it for the correct thread
                    self.unclaimed_responses.lock().push(resp);
                }
            }

            // 3. Check for timeout
            let now = crate::time::monotonic() as u64;
            if now.saturating_sub(start_time) > timeout {
                self.pending_requests.lock().retain(|(id, _)| *id != request_id);

                let mut stats = self.stats.lock();
                stats.timeouts += 1;
                stats.fallbacks_to_eevdf += 1;
                return None;
            }

            core::hint::spin_loop();
        }
    }

    /// Receives a request from the kernel (called by user-space policy).
    pub fn receive_request(&self) -> Option<ScxRequest> {
        self.request_queue.pop()
    }

    /// Sends a response back to the kernel (called by user-space policy).
    pub fn send_response(&self, response: ScxResponse) -> Result<(), ScxError> {
        self.response_queue
            .push(response)
            .map_err(|_| ScxError::ResponseQueueFull)
    }

    /// Checks if SCX is currently enabled and has an active policy.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed) && self.active_policy.lock().is_some()
    }

    /// Returns current SCX statistics.
    pub fn get_stats(&self) -> ScxStats {
        self.stats.lock().clone()
    }

    /// Monitors SCX policy responsiveness and triggers fallback if needed.
    ///
    /// This should be called periodically (e.g., every scheduler tick)
    /// to ensure the user-space policy is responding within timeout.
    pub fn check_health(&self) -> bool {
        if !self.is_enabled() {
            return true;
        }

        let now = crate::time::monotonic() as u64;
        let oldest = self.pending_requests.lock().first().copied();

        if let Some((_, ts)) = oldest {
            let timeout = self.timeout_ns.load(Ordering::Acquire);
            if now.saturating_sub(ts) > timeout {
                self.force_fallback();
                return false;
            }
        }
        true
    }

    /// Forces immediate fallback to EEVDF scheduler.
    pub fn force_fallback(&self) {
        self.enabled.store(false, Ordering::Release);
        {
            let mut stats = self.stats.lock();
            stats.fallbacks_to_eevdf += 1;
        }
        log::warn!("scx: watchdog detected timeout, forcing fallback to EEVDF");
        while self.request_queue.pop().is_some() {}
        while self.response_queue.pop().is_some() {}
        self.unclaimed_responses.lock().clear();
        self.pending_requests.lock().clear();
    }
}
