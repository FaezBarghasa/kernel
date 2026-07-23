#![forbid(unsafe_code)]

//! # IEEE 802.1AS gPTP Automotive Ethernet Time Synchronization
//!
//! Implements Generalized Precision Time Protocol (gPTP) for sub-microsecond clock sync
//! across Electronic Control Units (ECUs) over Automotive Ethernet.
//!
//! ## Mathematical & Synchronization Model
//! Given hardware NIC timestamps $t_1, t_2, t_3, t_4$:
//! $$\text{Delay} = \frac{(t_4 - t_1) - (t_3 - t_2)}{2}$$
//! $$\text{Offset} = (t_2 - t_1) - \text{Delay}$$

use core::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use spin::Mutex;

/// gPTP ECU Port Role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GptpRole {
    Grandmaster,
    Slave,
    Disabled,
}

/// 802.1AS High-Precision Timestamp (Nanoseconds and fractional sub-nanoseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PtpTimestamp {
    pub seconds: u64,
    pub nanoseconds: u32,
    pub sub_nanoseconds: u16,
}

impl PtpTimestamp {
    /// Converts timestamp into total nanoseconds.
    pub fn to_total_nanoseconds(&self) -> u64 {
        self.seconds
            .saturating_mul(1_000_000_000)
            .saturating_add(self.nanoseconds as u64)
    }

    /// Constructs a timestamp from nanoseconds.
    pub fn from_nanoseconds(nanos: u64) -> Self {
        Self {
            seconds: nanos / 1_000_000_000,
            nanoseconds: (nanos % 1_000_000_000) as u32,
            sub_nanoseconds: 0,
        }
    }
}

/// Peer Delay Measurement State.
#[derive(Debug, Clone, Copy, Default)]
pub struct PdelayState {
    pub t1_pdelay_req_sent: PtpTimestamp,
    pub t2_pdelay_req_received: PtpTimestamp,
    pub t3_pdelay_resp_sent: PtpTimestamp,
    pub t4_pdelay_resp_received: PtpTimestamp,
    pub peer_propagation_delay_ns: u64,
}

/// Generalized PTP Synchronization Engine.
pub struct GptpEngine {
    pub role: Mutex<GptpRole>,
    pub current_clock_offset_ns: AtomicI64,
    pub path_delay_ns: AtomicU64,
    pub last_sync_timestamp: AtomicU64,
    pub pdelay_state: Mutex<PdelayState>,
    pub clock_drift_ppb: AtomicI64, // Parts per billion drift compensation
}

impl GptpEngine {
    /// Creates a new `GptpEngine`.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub const fn new(role: GptpRole) -> Self {
        Self {
            role: Mutex::new(role),
            current_clock_offset_ns: AtomicI64::new(0),
            path_delay_ns: AtomicU64::new(0),
            last_sync_timestamp: AtomicU64::new(0),
            pdelay_state: Mutex::new(PdelayState {
                t1_pdelay_req_sent: PtpTimestamp { seconds: 0, nanoseconds: 0, sub_nanoseconds: 0 },
                t2_pdelay_req_received: PtpTimestamp { seconds: 0, nanoseconds: 0, sub_nanoseconds: 0 },
                t3_pdelay_resp_sent: PtpTimestamp { seconds: 0, nanoseconds: 0, sub_nanoseconds: 0 },
                t4_pdelay_resp_received: PtpTimestamp { seconds: 0, nanoseconds: 0, sub_nanoseconds: 0 },
                peer_propagation_delay_ns: 0,
            }),
            clock_drift_ppb: AtomicI64::new(0),
        }
    }

    /// Computes peer propagation delay: $\text{Delay} = \frac{(t_4 - t_1) - (t_3 - t_2)}{2}$.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn calculate_peer_delay(
        &self,
        t1: PtpTimestamp,
        t2: PtpTimestamp,
        t3: PtpTimestamp,
        t4: PtpTimestamp,
    ) -> u64 {
        let t1_ns = t1.to_total_nanoseconds();
        let t2_ns = t2.to_total_nanoseconds();
        let t3_ns = t3.to_total_nanoseconds();
        let t4_ns = t4.to_total_nanoseconds();

        let req_diff = t4_ns.saturating_sub(t1_ns);
        let resp_diff = t3_ns.saturating_sub(t2_ns);

        let delay = req_diff.saturating_sub(resp_diff) / 2;

        let mut lock = self.pdelay_state.lock();
        lock.t1_pdelay_req_sent = t1;
        lock.t2_pdelay_req_received = t2;
        lock.t3_pdelay_resp_sent = t3;
        lock.t4_pdelay_resp_received = t4;
        lock.peer_propagation_delay_ns = delay;

        self.path_delay_ns.store(delay, Ordering::Release);
        delay
    }

    /// Computes master-to-slave clock offset: $\text{Offset} = (t_2 - t_1) - \text{Delay}$.
    ///
    /// Complexity: $\mathcal{O}(1)$
    pub fn calculate_clock_offset(
        &self,
        t1_master_tx: PtpTimestamp,
        t2_slave_rx: PtpTimestamp,
        peer_delay_ns: u64,
    ) -> i64 {
        let t1_ns = t1_master_tx.to_total_nanoseconds() as i64;
        let t2_ns = t2_slave_rx.to_total_nanoseconds() as i64;

        let diff = t2_ns - t1_ns;
        let offset = diff - (peer_delay_ns as i64);

        self.current_clock_offset_ns.store(offset, Ordering::Release);
        self.last_sync_timestamp.store(t2_slave_rx.to_total_nanoseconds(), Ordering::Release);

        // Simple Proportional Servo adjustment for drift tracking
        let prev_drift = self.clock_drift_ppb.load(Ordering::Acquire);
        let proportional_adjustment = offset / 16;
        self.clock_drift_ppb.store(prev_drift.saturating_add(proportional_adjustment), Ordering::Release);

        offset
    }

    /// Synchronizes a local hardware timestamp using current estimated offset.
    /// Returns hardware time synchronized with ECU Master clock in sub-microsecond precision.
    pub fn synchronize_local_time(&self, raw_local_ns: u64) -> u64 {
        let offset = self.current_clock_offset_ns.load(Ordering::Acquire);
        if offset >= 0 {
            raw_local_ns.saturating_add(offset as u64)
        } else {
            raw_local_ns.saturating_sub(offset.unsigned_abs())
        }
    }

    /// Returns `true` if current synchronization precision is within sub-microsecond bound ($\le 1000 \text{ ns}$).
    pub fn is_synchronized(&self) -> bool {
        let offset = self.current_clock_offset_ns.load(Ordering::Acquire).abs();
        offset <= 1000
    }
}

/// Global gPTP Engine instance (default Slave role).
pub static GPTP_ENGINE: GptpEngine = GptpEngine::new(GptpRole::Slave);
