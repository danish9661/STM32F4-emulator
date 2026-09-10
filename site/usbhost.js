// In-browser USB host for the usb_cdc_test demo preset: plays the same
// scripted enumeration + echo as site/test_usb.mjs, but event-driven —
// one `frame(uartText)` call per rAF from the page loop advances the
// script when the previous step completes. No gateway involved (USB has
// no gateway backend; this is the netsim equivalent for USB).
//
// Phases mirror the node test: reset -> enum-done -> GET_DESCRIPTOR
// (device/config/string) -> SET_ADDRESS -> SET_CONFIGURATION ->
// SET_CONTROL_LINE_STATE -> SET_LINE_CODING -> 2x bulk echo. Takes block
// phase advance until the firmware answers; status-stage ZLPs are
// fire-and-forget (the firmware consumes them without replying).

const SETUPS = {
    dev: [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00],
    cfg: [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00],
    str: [0x80, 0x06, 0x00, 0x03, 0x00, 0x00, 0xFF, 0x00],
    addr: [0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00],
    conf: [0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
    lineState: [0x21, 0x22, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00],
    lineCoding: [0x21, 0x20, 0, 0, 7, 0, 0, 0],
};
const LINE_CODING_BYTES = [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08];
const ECHO1 = [72, 101, 108, 108, 111, 85, 83, 66]; // HelloUSB
const ECHO2 = [87, 111, 114, 108, 100, 33, 33, 33]; // World!!!

export function createUsbHost(bindings) {
    const U8 = (a) => Uint8Array.from(a);
    // Script: each entry runs once, in order. `take` entries poll until
    // the IN blob arrives (STALl aborts the run loudly).
    const script = [
        { waitUart: 'USB init done' },
        { fire: () => bindings.usb_reset() },
        { waitUart: 'USBRST' },
        { fire: () => bindings.usb_enumerated() },
        { waitUart: 'ENUMDNE' },
        { setup: SETUPS.dev }, { take: 0 }, { out: { ep: 0, data: [] } },
        { setup: SETUPS.cfg }, { take: 0 }, { out: { ep: 0, data: [] } },
        { setup: SETUPS.str }, { take: 0 }, { out: { ep: 0, data: [] } },
        { setup: SETUPS.addr }, { take: 0 },
        { setup: SETUPS.conf }, { take: 0 },
        { waitUart: 'USB enum done' },
        { setup: SETUPS.lineState }, { take: 0 },
        { setup: SETUPS.lineCoding },
        { out: { ep: 0, data: LINE_CODING_BYTES } }, { take: 0 },
        { out: { ep: 1, data: ECHO1 } }, { take: 1 },
        { out: { ep: 1, data: ECHO2 } }, { take: 1 },
    ];
    let pc = 0;
    let done = false;

    const frame = (uartText) => {
        if (done || pc >= script.length) { done = true; return; }
        const op = script[pc];
        if (op.waitUart) {
            if (uartText.includes(op.waitUart)) pc++;
            return;
        }
        if (op.fire) { try { op.fire(); } catch (e) {} pc++; return; }
        if (op.setup) {
            try { bindings.usb_inject_setup(U8(op.setup)); } catch (e) {}
            pc++;
            return;
        }
        if (op.out) {
            try { bindings.usb_inject_out(op.out.ep, U8(op.out.data)); } catch (e) {}
            pc++;
            return;
        }
        if (op.take !== undefined) {
            let st = 0;
            try { st = bindings.usb_in_status(op.take); } catch (e) {}
            if (st === 2) { done = true; return; } // STALL: stop driving
            if (st === 1) {
                try { bindings.usb_take_in(op.take); } catch (e) {}
                pc++;
            }
            return;
        }
        pc++;
    };

    return { frame, get done() { return done; } };
}
