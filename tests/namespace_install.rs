#![forbid(unsafe_code)]

use clap::Parser;
use geesefs_rs::{config::Config, error::Error, inode::Inode, namespace::Namespace};

fn install(namespace: &mut Namespace, config: &Config, name: &str) -> u64 {
    namespace
        .install(name, |ino| {
            Inode::new(ino, name.into(), false, None, config)
        })
        .unwrap_or_else(|error| {
            panic!("first synchronous install of {name} must succeed: {error:?}")
        })
        .ino
}

fn full_namespace(pins: usize) -> (Namespace, Config, Vec<u64>, u64) {
    let config = Config::try_parse_from(["test", "bucket", "/tmp/unused-mount"]).unwrap();
    assert_eq!(config.metadata_limit, None);
    let mut namespace = Namespace::new(Inode::new(1, "".into(), true, None, &config), pins + 2);
    let mut pinned = Vec::new();
    for index in 0..pins {
        let ino = install(&mut namespace, &config, &format!("pinned-{index}"));
        namespace.get(ino).unwrap().state.lock().unwrap().lookups = 1;
        pinned.push(ino);
    }
    let victim = install(&mut namespace, &config, "victim");
    (namespace, config, pinned, victim)
}

#[test]
fn synchronous_install_reaches_an_eligible_tail_beyond_multiple_pinned_batches() {
    for pins in [129, 257, 385] {
        let (mut namespace, config, pinned, victim) = full_namespace(pins);
        let replacement = install(&mut namespace, &config, "replacement");
        assert!(namespace.get(victim).is_err());
        assert!(namespace.named("victim").is_none());
        assert_eq!(namespace.named("replacement").unwrap().ino, replacement);
        assert_eq!(namespace.evictions, 1);
        assert_eq!(namespace.capacity_failures, 0);
        for ino in pinned {
            assert_eq!(namespace.get(ino).unwrap().ino, ino);
        }
    }
}

#[test]
fn synchronous_install_reports_enomem_after_a_finite_all_pinned_scan() {
    let (mut namespace, _config, pinned, victim) = full_namespace(385);
    namespace.get(victim).unwrap().state.lock().unwrap().lookups = 1;
    for attempt in 1..=3 {
        let result = namespace.install("rejected", |_| {
            panic!("exhausted admission must not construct an inode")
        });
        assert!(
            matches!(result, Err(Error::Fs(libc::ENOMEM))),
            "all-pinned synchronous admission must finish with ENOMEM, not retryable EAGAIN"
        );
        assert_eq!(namespace.capacity_failures, attempt);
        assert_eq!(namespace.evictions, 0);
        assert_eq!(namespace.nodes.len(), 387);
        assert!(namespace.named("rejected").is_none());
    }
    for ino in pinned.into_iter().chain([victim]) {
        assert_eq!(namespace.get(ino).unwrap().ino, ino);
    }
}

#[test]
fn synchronous_install_revisits_a_released_prefix_after_previous_success() {
    let (mut namespace, config, pinned, victim) = full_namespace(129);
    let replacement = install(&mut namespace, &config, "replacement");
    namespace
        .get(replacement)
        .unwrap()
        .state
        .lock()
        .unwrap()
        .lookups = 1;
    namespace
        .get(pinned[0])
        .unwrap()
        .state
        .lock()
        .unwrap()
        .lookups = 0;
    let next = install(&mut namespace, &config, "next");
    assert!(namespace.get(victim).is_err());
    assert!(namespace.get(pinned[0]).is_err());
    assert_eq!(namespace.evictions, 2);
    assert_eq!(namespace.capacity_failures, 0);
    for ino in pinned.into_iter().skip(1).chain([replacement]) {
        assert_eq!(namespace.get(ino).unwrap().ino, ino);
    }
    let existing = namespace
        .install("next", |_| {
            panic!("existing binding must not be constructed again")
        })
        .unwrap();
    assert_eq!(existing.ino, next);
    assert_eq!(namespace.evictions, 2);
}
