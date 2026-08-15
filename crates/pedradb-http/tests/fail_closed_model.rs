//! Stateright model over the **real** [`pedradb_http`] fail-closed kernel
//! (F102 / F104 / F105 / F153 / F154 / F157 / F158 / F159).
//!
//! Faithfulness: whether a parse error writes a status, TE is rejected, a
//! present bad int is an error, LF header breaks, and Expect: 100-continue
//! are production `fail_closed`. The socket write is an axiom.
//!
//! - **Inv-no-mute:** parse error writes [`parse_error_status`] (F102).
//! - **Inv-te-rejected:** Transfer-Encoding is not accepted (F104).
//! - **Inv-bad-int:** `ttl_ms=abc` does not take the default (F105).
//! - **Inv-lf-framed:** `\n\n` is a header break (F153).
//! - **Inv-expect-100:** `Expect: 100-continue` is honoured (F154).
//! - **Inv-http11-host:** HTTP/1.1 requires Host (F157).
//! - **Inv-empty-host:** empty `Host:` is invalid (F158).
//! - **Inv-expect-known:** unknown Expect is 417 (F159).
//! - AS-IS mutants must produce a counterexample (teeth).

use pedradb_http::{
    expect_field_ok, expect_field_ok_as_is, expects_100_continue, expects_100_continue_as_is,
    header_break_end, header_break_end_as_is, host_value_ok, host_value_ok_as_is,
    http_version_requires_host,
    http_version_requires_host_as_is, parse_error_status, parse_error_writes_status,
    parse_error_writes_status_as_is, present_bad_int_is_error, present_bad_int_is_error_as_is,
    reject_transfer_encoding, reject_transfer_encoding_as_is,
};
use stateright::{Checker, Model, Property};

/// LF-only request that FIXED frames and AS-IS misses (F153).
const LF_REQ: &[u8] = b"PUT /kv/x HTTP/1.0\nContent-Length: 2\n\nok";

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
#[allow(clippy::struct_excessive_bools)]
struct St {
    mute: bool,
    te_accepted: bool,
    bad_int_defaulted: bool,
    lf_unparsed: bool,
    expect_deadlock: bool,
    http11_host_skipped: bool,
    empty_host_ok: bool,
    unknown_expect_ok: bool,
    saw_parse: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Act {
    ParseErr,
    TransferEncoding,
    BadInt,
    LfHeaders,
    Expect100,
    Http11Host,
    EmptyHost,
    UnknownExpect,
}

#[derive(Clone)]
struct FailClosedModel {
    fixed: bool,
}

impl FailClosedModel {
    fn writes_status(&self) -> bool {
        if self.fixed {
            parse_error_writes_status()
        } else {
            parse_error_writes_status_as_is()
        }
    }

    fn reject_te(&self) -> bool {
        if self.fixed {
            reject_transfer_encoding()
        } else {
            reject_transfer_encoding_as_is()
        }
    }

    fn bad_int_err(&self) -> bool {
        if self.fixed {
            present_bad_int_is_error()
        } else {
            present_bad_int_is_error_as_is()
        }
    }

    fn break_end(&self, buf: &[u8]) -> Option<usize> {
        if self.fixed {
            header_break_end(buf)
        } else {
            header_break_end_as_is(buf)
        }
    }

    fn expect_100(&self, v: &str) -> bool {
        if self.fixed {
            expects_100_continue(v)
        } else {
            expects_100_continue_as_is(v)
        }
    }
}

impl Model for FailClosedModel {
    type State = St;
    type Action = Act;

    fn init_states(&self) -> Vec<Self::State> {
        vec![St {
            mute: false,
            te_accepted: false,
            bad_int_defaulted: false,
            lf_unparsed: false,
            expect_deadlock: false,
            http11_host_skipped: false,
            empty_host_ok: false,
            unknown_expect_ok: false,
            saw_parse: false,
        }]
    }

    fn actions(&self, _state: &Self::State, actions: &mut Vec<Self::Action>) {
        actions.extend([
            Act::ParseErr,
            Act::TransferEncoding,
            Act::BadInt,
            Act::LfHeaders,
            Act::Expect100,
            Act::Http11Host,
            Act::EmptyHost,
            Act::UnknownExpect,
        ]);
    }

    fn next_state(&self, state: &Self::State, action: Self::Action) -> Option<Self::State> {
        let mut next = state.clone();
        match action {
            Act::ParseErr => {
                next.saw_parse = true;
                if self.writes_status() {
                    // Axiom: caller writes `parse_error_status()` (400).
                    let _ = parse_error_status();
                } else {
                    next.mute = true;
                }
            }
            Act::TransferEncoding => {
                if !self.reject_te() {
                    next.te_accepted = true;
                }
            }
            Act::BadInt => {
                if !self.bad_int_err() {
                    next.bad_int_defaulted = true;
                }
            }
            Act::LfHeaders => {
                if self.break_end(LF_REQ).is_none() {
                    next.lf_unparsed = true;
                }
            }
            Act::Expect100 => {
                if !self.expect_100("100-continue") {
                    next.expect_deadlock = true;
                }
            }
            Act::Http11Host => {
                let req = if self.fixed {
                    http_version_requires_host("HTTP/1.1")
                } else {
                    http_version_requires_host_as_is("HTTP/1.1")
                };
                if !req {
                    next.http11_host_skipped = true;
                }
            }
            Act::EmptyHost => {
                let ok = if self.fixed {
                    host_value_ok("")
                } else {
                    host_value_ok_as_is("")
                };
                if ok {
                    next.empty_host_ok = true;
                }
            }
            Act::UnknownExpect => {
                let ok = if self.fixed {
                    expect_field_ok("blah")
                } else {
                    expect_field_ok_as_is("blah")
                };
                if ok {
                    next.unknown_expect_ok = true;
                }
            }
        }
        Some(next)
    }

    fn properties(&self) -> Vec<Property<Self>> {
        vec![
            Property::always("Inv-no-mute", inv_no_mute),
            Property::always("Inv-te-rejected", inv_te_rejected),
            Property::always("Inv-bad-int", inv_bad_int),
            Property::always("Inv-lf-framed", inv_lf_framed),
            Property::always("Inv-expect-100", inv_expect_100),
            Property::always("Inv-http11-host", inv_http11_host),
            Property::always("Inv-empty-host", inv_empty_host),
            Property::always("Inv-expect-known", inv_expect_known),
            Property::sometimes("non-vacuity-parse", non_vacuity_parse),
        ]
    }
}

fn inv_no_mute(_: &FailClosedModel, s: &St) -> bool {
    !s.mute
}

fn inv_te_rejected(_: &FailClosedModel, s: &St) -> bool {
    !s.te_accepted
}

fn inv_bad_int(_: &FailClosedModel, s: &St) -> bool {
    !s.bad_int_defaulted
}

fn inv_lf_framed(_: &FailClosedModel, s: &St) -> bool {
    !s.lf_unparsed
}

fn inv_expect_100(_: &FailClosedModel, s: &St) -> bool {
    !s.expect_deadlock
}

fn inv_http11_host(_: &FailClosedModel, s: &St) -> bool {
    !s.http11_host_skipped
}

fn inv_empty_host(_: &FailClosedModel, s: &St) -> bool {
    !s.empty_host_ok
}

fn inv_expect_known(_: &FailClosedModel, s: &St) -> bool {
    !s.unknown_expect_ok
}

fn non_vacuity_parse(_: &FailClosedModel, s: &St) -> bool {
    s.saw_parse
}

#[test]
fn fixed_fail_closed_holds() {
    let checker = FailClosedModel { fixed: true }.checker().spawn_bfs().join();
    checker.assert_properties();
}

#[test]
fn as_is_mutes_parse_error() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-no-mute").is_some(),
        "AS-IS parse error must drop the socket mute (F102 teeth)"
    );
}

#[test]
fn as_is_accepts_te() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-te-rejected").is_some(),
        "AS-IS must accept Transfer-Encoding (F104 teeth)"
    );
}

#[test]
fn as_is_defaults_bad_int() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-bad-int").is_some(),
        "AS-IS must default ttl_ms=abc (F105 teeth)"
    );
}

#[test]
fn as_is_misses_lf_break() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-lf-framed").is_some(),
        "AS-IS must miss LF-only header break (F153 teeth)"
    );
}

#[test]
fn as_is_skips_expect_100() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-expect-100").is_some(),
        "AS-IS must ignore Expect: 100-continue (F154 teeth)"
    );
}

#[test]
fn as_is_skips_http11_host() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-http11-host").is_some(),
        "AS-IS must not require Host on HTTP/1.1 (F157 teeth)"
    );
}

#[test]
fn as_is_accepts_empty_host() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-empty-host").is_some(),
        "AS-IS must treat empty Host as present (F158 teeth)"
    );
}

#[test]
fn as_is_accepts_unknown_expect() {
    let checker = FailClosedModel { fixed: false }
        .checker()
        .spawn_bfs()
        .join();
    assert!(
        checker.discovery("Inv-expect-known").is_some(),
        "AS-IS must ignore unknown Expect (F159 teeth)"
    );
}
