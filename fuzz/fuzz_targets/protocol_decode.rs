//! Wire-protocol decoder under libFuzzer.
//!
//! Properties checked on every input:
//!   * no decode function panics on any byte string (including every prefix),
//!   * a message that decodes re-encodes to bytes that decode to the same
//!     message (decode is a left-inverse of encode on its image),
//!   * the re-encoded bytes are exactly the canonical message size.
#![no_main]
use ferrox::protocol::{
    CANCEL_ORDER_SIZE, EXECUTION_REPORT_SIZE, EngineCommand, NEW_ORDER_SIZE, decode_execution_report,
    decode_message, encode_cancel_order, encode_new_order, message_size,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Every prefix: the gateway frames by message_size(first byte), so short
    // and over-long buffers both reach the decoder in practice.
    for len in 0..=data.len().min(64) {
        let _ = decode_message(&data[..len]);
        let _ = decode_execution_report(&data[..len]);
    }
    if let Some(&t) = data.first() {
        let _ = message_size(t);
    }

    match decode_message(data) {
        Ok(EngineCommand::NewOrder(order)) => {
            let mut buf = [0u8; NEW_ORDER_SIZE];
            let n = encode_new_order(&mut buf, &order).expect("encode of a decoded order");
            assert_eq!(n, NEW_ORDER_SIZE);
            let again = decode_message(&buf[..n]).expect("re-decode");
            assert_eq!(again, EngineCommand::NewOrder(order));
        }
        Ok(EngineCommand::CancelOrder { order_id }) => {
            let mut buf = [0u8; CANCEL_ORDER_SIZE];
            let n = encode_cancel_order(&mut buf, order_id).expect("encode of a decoded cancel");
            assert_eq!(n, CANCEL_ORDER_SIZE);
            let again = decode_message(&buf[..n]).expect("re-decode");
            assert_eq!(again, EngineCommand::CancelOrder { order_id });
        }
        Err(_) => {}
    }

    if let Ok(report) = decode_execution_report(data) {
        // Execution reports are only ever encoded from a Fill; check the
        // decoder is total and stable on its own output instead.
        assert!(data.len() >= EXECUTION_REPORT_SIZE);
        let again = decode_execution_report(&data[..EXECUTION_REPORT_SIZE]).unwrap();
        assert_eq!(again, report);
    }
});
