// The hardening changes the whole test process, so one test checks every
// flag of this system after applying it.

#[test]
fn core_dumps_and_tracing_are_turned_off() {
    super::apply();

    let core = rustix::process::getrlimit(rustix::process::Resource::Core);
    assert_eq!(core.current, Some(0));
    #[cfg(target_os = "linux")]
    assert_eq!(
        rustix::process::dumpable_behavior().unwrap(),
        rustix::process::DumpableBehavior::NotDumpable
    );
    #[cfg(target_os = "freebsd")]
    assert_eq!(
        rustix::process::trace_status(None).unwrap(),
        rustix::process::TracingStatus::NotTraceble
    );
}
