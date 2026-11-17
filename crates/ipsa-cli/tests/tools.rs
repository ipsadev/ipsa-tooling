use ipsa_cli::tools::docker;

#[test]
fn compose_argv_prepends_profiles() {
    assert_eq!(
        docker::compose_argv(&["up", "-d", "cosmos"]),
        vec![
            "compose",
            "--profile",
            "local",
            "--profile",
            "ibcv2",
            "--profile",
            "watcher",
            "--profile",
            "verifier",
            "up",
            "-d",
            "cosmos"
        ]
    );
}

#[test]
fn compose_argv_with_no_extra_is_just_the_prefix() {
    assert_eq!(
        docker::compose_argv(&[]),
        vec![
            "compose",
            "--profile",
            "local",
            "--profile",
            "ibcv2",
            "--profile",
            "watcher",
            "--profile",
            "verifier"
        ]
    );
}
