#![cfg(test)]

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use std::thread;
use std::time::Instant;

use crate::sched::context_ring::ContextRing;
use crate::sched::eevdf_math::{calculate_lag, calculate_vdeadline, is_task_eligible, update_vruntime};
use crate::sched::eevdf_types::{EevdfTask, TaskState};
use crate::sched::scx_bridge::{ScxBridge, ScxStats};
use crate::sched::scx_types::{ScxOperation, ScxPolicyInfo, ScxResponse, ScxResult, ScxTaskData};

// =============================================================================
// EEVDF Math Tests
// =============================================================================

#[test]
fn test_vdeadline_calculation_basic() {
    // Test case: vruntime=1000, slice=100ns, weight=512
    // Expected: 1000 + (100 << 10) / 512 = 1000 + 102400 / 512 = 1000 + 200 = 1200
    let result = calculate_vdeadline(1000, 100, 512);
    assert_eq!(result, 1200);
}

#[test]
fn test_vdeadline_calculation_high_weight() {
    // High weight task should have smaller deadline increment
    let result = calculate_vdeadline(1000, 100, 1024);
    assert_eq!(result, 1100); // 1000 + 100
}

#[test]
fn test_vdeadline_calculation_low_weight() {
    // Low weight task should have larger deadline increment
    let result = calculate_vdeadline(1000, 100, 1);
    assert_eq!(result, 1000 + (100 << 10)); // 1000 + 102400
}

#[test]
fn test_vruntime_update() {
    let initial_vruntime = 5000;
    let delta_ns = 1000;
    let weight = 512;

    let updated = update_vruntime(initial_vruntime, delta_ns, weight);
    // Expected: 5000 + (1000 * 1024) / 512 = 5000 + 2000 = 7000
    assert_eq!(updated, 7000);
}

#[test]
fn test_task_eligibility() {
    assert!(is_task_eligible(1000, 1500)); // deadline < current
    assert!(is_task_eligible(1000, 1000)); // deadline == current
    assert!(!is_task_eligible(1500, 1000)); // deadline > current
}

#[test]
fn test_lag_calculation() {
    assert_eq!(calculate_lag(1000, 1500), 500); // Positive lag (overdue)
    assert_eq!(calculate_lag(1500, 1000), -500); // Negative lag (ahead)
    assert_eq!(calculate_lag(1000, 1000), 0); // No lag
}

#[test]
fn test_overflow_protection() {
    // Test that overflow doesn't panic
    let result = calculate_vdeadline(u64::MAX - 1000, 100, 1);
    assert_eq!(result, u64::MAX); // Should saturate
}

#[test]
#[should_panic(expected = "weight cannot be zero")]
fn test_zero_weight_panics() {
    calculate_vdeadline(1000, 100, 0);
}

// =============================================================================
// Context Ring Tests
// =============================================================================

#[test]
fn test_ring_creation() {
    let ring = ContextRing::new(100);
    assert_eq!(ring.capacity(), 100);
    assert_eq!(ring.len(), 0);
    assert!(ring.is_empty());
}

#[test]
fn test_insert_single_task() {
    let ring = ContextRing::new(10);
    let task = EevdfTask {
        task_id: 1,
        vruntime: 1000,
        vdeadline: 2000,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };

    assert!(ring.insert(task).is_ok());
    assert_eq!(ring.len(), 1);
}

#[test]
fn test_insert_full_ring() {
    let ring = ContextRing::new(2);

    let task1 = EevdfTask {
        task_id: 1,
        vruntime: 1000,
        vdeadline: 2000,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };
    let task2 = EevdfTask {
        task_id: 2,
        vruntime: 1100,
        vdeadline: 2100,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };
    let task3 = EevdfTask {
        task_id: 3,
        vruntime: 1200,
        vdeadline: 2200,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };

    assert!(ring.insert(task1).is_ok());
    assert!(ring.insert(task2).is_ok());

    // Third insert should fail and return the task
    let result = ring.insert(task3);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().task_id, 3);
}

#[test]
fn test_select_next_empty() {
    let ring = ContextRing::new(10);
    assert!(ring.select_next().is_none());
}

#[test]
fn test_select_next_earliest_deadline() {
    let ring = ContextRing::new(10);

    let task1 = EevdfTask {
        task_id: 1,
        vruntime: 1000,
        vdeadline: 3000,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };
    let task2 = EevdfTask {
        task_id: 2,
        vruntime: 1000,
        vdeadline: 1000,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };
    let task3 = EevdfTask {
        task_id: 3,
        vruntime: 1000,
        vdeadline: 2000,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };

    ring.insert(task1).unwrap();
    ring.insert(task2).unwrap();
    ring.insert(task3).unwrap();

    let selected = ring.select_next().unwrap();
    assert_eq!(selected.task_id, 2); // Earliest deadline
}

#[test]
fn test_remove_task() {
    let ring = ContextRing::new(10);
    let task = EevdfTask {
        task_id: 42,
        vruntime: 1000,
        vdeadline: 2000,
        slice_ns: 100,
        weight: 512,
        state: TaskState::Ready,
        eligible_since: 0,
        cpu_affinity: 0xFF,
    };

    ring.insert(task).unwrap();
    assert_eq!(ring.len(), 1);

    let removed = ring.remove(42);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().task_id, 42);
    assert_eq!(ring.len(), 0);
}

#[test]
fn test_remove_nonexistent_task() {
    let ring = ContextRing::new(10);
    assert!(ring.remove(999).is_none());
}

#[test]
fn test_concurrent_insert() {
    let ring = Arc::new(ContextRing::new(1000));
    let mut handles = vec![];

    for i in 0..10 {
        let ring_clone = Arc::clone(&ring);
        let handle = thread::spawn(move || {
            for j in 0..100 {
                let task = EevdfTask {
                    task_id: i * 100 + j,
                    vruntime: 1000,
                    vdeadline: 2000,
                    slice_ns: 100,
                    weight: 512,
                    state: TaskState::Ready,
                    eligible_since: 0,
                    cpu_affinity: 0xFF,
                };
                let _ = ring_clone.insert(task);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert_eq!(ring.len(), 1000);
}

// =============================================================================
// SCX Bridge Tests
// =============================================================================

#[test]
fn test_scx_bridge_creation() {
    let bridge = ScxBridge::new(100);
    assert!(!bridge.is_enabled());
}

#[test]
fn test_register_policy() {
    let bridge = ScxBridge::new(100);
    let policy = ScxPolicyInfo {
        name: "test_policy".into(),
        version: "1.0".into(),
        pid: 1234,
        timeout_ns: 50_000,
        is_active: true,
    };

    assert!(bridge.register_policy(policy).is_ok());
    assert!(bridge.is_enabled());
}

#[test]
fn test_register_duplicate_policy() {
    let bridge = ScxBridge::new(100);
    let policy1 = ScxPolicyInfo {
        name: "test_policy_1".into(),
        version: "1.0".into(),
        pid: 1234,
        timeout_ns: 50_000,
        is_active: true,
    };
    let policy2 = ScxPolicyInfo {
        name: "test_policy_2".into(),
        version: "1.0".into(),
        pid: 1234,
        timeout_ns: 50_000,
        is_active: true,
    };

    bridge.register_policy(policy1).unwrap();
    assert!(matches!(
        bridge.register_policy(policy2),
        Err(crate::sched::scx_types::ScxError::PolicyAlreadyRegistered)
    ));
}

#[test]
fn test_send_request_timeout() {
    let bridge = ScxBridge::new(100);
    let policy = ScxPolicyInfo {
        name: "timeout_policy".into(),
        version: "1.0".into(),
        pid: 1234,
        timeout_ns: 1000, // Very small timeout
        is_active: true,
    };
    bridge.register_policy(policy).unwrap();

    let operation = ScxOperation::SelectNext { eligible_tasks: vec![] };
    let task_data = ScxTaskData::default();

    // Don't send a response, should timeout
    let result = bridge.send_request(operation, task_data);
    assert!(result.is_none()); // Timeout occurred
}

#[test]
fn test_send_request_success() {
    let bridge = Arc::new(ScxBridge::new(100));
    let policy = ScxPolicyInfo {
        name: "success_policy".into(),
        version: "1.0".into(),
        pid: 1234,
        timeout_ns: 10_000_000, // Large timeout
        is_active: true,
    };
    bridge.register_policy(policy).unwrap();

    let bridge_clone = Arc::clone(&bridge);

    // Spawn thread to simulate user-space policy
    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(10));
        if let Some(request) = bridge_clone.receive_request() {
            let response = ScxResponse {
                request_id: request.request_id,
                result: ScxResult::Selected { task_id: 42 },
                processing_time_ns: 10_000,
            };
            bridge_clone.send_response(response).unwrap();
        }
    });

    let operation = ScxOperation::SelectNext { eligible_tasks: vec![] };
    let task_data = ScxTaskData::default();

    let result = bridge.send_request(operation, task_data);
    assert!(result.is_some());
    assert!(matches!(
        result.unwrap().result,
        ScxResult::Selected { task_id: 42 }
    ));
}

#[test]
fn test_health_check_fallback() {
    let bridge = ScxBridge::new(100);
    let policy = ScxPolicyInfo {
        name: "fallback_policy".into(),
        version: "1.0".into(),
        pid: 1234,
        timeout_ns: 1000, // Small timeout
        is_active: true,
    };
    bridge.register_policy(policy).unwrap();

    // Send request but don't respond
    let operation = ScxOperation::SelectNext { eligible_tasks: vec![] };
    let task_data = ScxTaskData::default();

    // We spawn it in another thread or run asynchronously so it times out
    let bridge_arc = Arc::new(bridge);
    let bridge_clone = Arc::clone(&bridge_arc);

    thread::spawn(move || {
        let _ = bridge_clone.send_request(operation, task_data);
    });

    // Wait for timeout
    thread::sleep(std::time::Duration::from_millis(10));

    // Health check should detect timeout
    assert!(!bridge_arc.check_health());
}

// =============================================================================
// Benchmarks (run as tests to verify functionality and performance)
// =============================================================================

#[test]
fn bench_vdeadline_calculation() {
    let iterations = 1_000_000;
    let start = Instant::now();

    for i in 0..iterations {
        let _ = calculate_vdeadline(i, 100, 512);
    }

    let elapsed = start.elapsed();
    let per_op = elapsed / iterations as u32;

    println!("calculate_vdeadline: {:?} per operation", per_op);
    assert!(per_op.as_nanos() < 10, "Should complete in < 10ns");
}

#[test]
fn bench_vruntime_update() {
    let iterations = 1_000_000;
    let start = Instant::now();

    for i in 0..iterations {
        let _ = update_vruntime(i, 1000, 512);
    }

    let elapsed = start.elapsed();
    let per_op = elapsed / iterations as u32;

    println!("update_vruntime: {:?} per operation", per_op);
    assert!(per_op.as_nanos() < 10, "Should complete in < 10ns");
}

#[test]
fn bench_insert_throughput() {
    let ring = ContextRing::new(100_000);
    let iterations = 100_000;

    let start = Instant::now();
    for i in 0..iterations {
        let task = EevdfTask {
            task_id: i,
            vruntime: i * 1000,
            vdeadline: i * 2000,
            slice_ns: 100,
            weight: 512,
            state: TaskState::Ready,
            eligible_since: 0,
            cpu_affinity: 0xFF,
        };
        let _ = ring.insert(task);
    }
    let elapsed = start.elapsed();

    let per_op = elapsed / iterations as u32;
    println!("insert: {:?} per operation", per_op);
    assert!(per_op.as_nanos() < 100, "Should complete in < 100ns");
}

#[test]
fn bench_select_next_performance() {
    let ring = ContextRing::new(10_000);

    // Fill ring with 10,000 tasks
    for i in 0..10_000 {
        let task = EevdfTask {
            task_id: i,
            vruntime: i * 100,
            vdeadline: i * 1000,
            slice_ns: 100,
            weight: 512,
            state: TaskState::Ready,
            eligible_since: 0,
            cpu_affinity: 0xFF,
        };
        ring.insert(task).unwrap();
    }

    let iterations = 100_000;
    let start = Instant::now();

    for _ in 0..iterations {
        let _ = ring.select_next();
    }

    let elapsed = start.elapsed();
    let per_op = elapsed / iterations as u32;

    println!("select_next with 10k tasks: {:?} per operation", per_op);
    assert!(per_op.as_nanos() < 200, "Should complete in < 200ns");
}
