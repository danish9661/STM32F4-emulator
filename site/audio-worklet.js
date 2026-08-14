// AudioWorklet for the DOOM page: resamples the guest's 11025 Hz mono stream
// to the context sample rate and plays it continuously on the audio thread.
//
// The hard problem here is that the guest does NOT produce audio at wall-clock
// rate.  DOOM mixes exactly one frame's worth of samples (11025/35 = 315) per
// RENDERED frame, and the emulator sustains ~22 fps rather than DOOM's native
// 35 — so production runs at only ~60-70% of what realtime playback consumes.
//
// The previous policy (emit silence on underrun AND flush the queue) turned
// that permanent deficit into permanent crackle: play a fragment, starve,
// discard whatever had arrived, repeat.  Discarding was the worst part — it
// threw away good audio, so the output was almost entirely gaps.
//
// Instead we do dynamic rate control, the standard emulator-audio approach:
// track how much audio is buffered and continuously nudge the playback rate
// so consumption matches production.  When the guest runs at 63% of realtime
// the stream plays ~37% slow (pitched down) but CONTINUOUS and recognizable —
// which also matches what the eye sees, since the game itself is in slow
// motion.  When the guest keeps up, the rate sits at exactly 1.0 and pitch is
// correct.  Nothing is ever flushed.
const MAX_QUEUE = 8192;         // ~0.74 s at 11025 Hz — hard latency bound
const TARGET_Q = 3072;          // ~0.28 s: the depth the controller aims for.
                                // Production is bursty (one 315-sample block
                                // per rendered frame, ~26/s), so the cushion
                                // must cover a couple of missed frames or the
                                // buffer dips empty between bursts and clicks.
// Floor for the rate controller.  It must sit BELOW the slowest rate the
// guest can force on us, or the floor itself causes dropouts: required rate
// == fps/35, so 24 fps needs 0.69 but a dip to 14 fps needs 0.40.  A 0.5
// floor therefore guaranteed starvation on every dip.  0.28 covers ~10 fps.
const RATIO_MIN = 0.28;
const RATIO_MAX = 1.25;         // ceiling: drain a backlog without chipmunking
const SMOOTH = 0.02;            // per-block filter (~0.13 s) — avoids warble
const TRIM_AT = 4096;           // compact the queue after this many consumed

class DoomAudio extends AudioWorkletProcessor {
    constructor() {
        super();
        this.q = new Float32Array(0);   // pending 11025 Hz samples
        this.pos = 0;                   // read cursor in 11025 Hz space
        // Resample step: how far the 11025 Hz read cursor advances per OUTPUT
        // sample.  This is SRC/DST (11025/48000 ≈ 0.23), NOT DST/SRC — the
        // inverted form (sampleRate/11025 ≈ 4.35) consumed input ~19x too
        // fast, playing every sound far above audible pitch and then starving
        // instantly.  That was the real cause of the long-standing "audio is
        // only crackling" report: the old tests only counted samples the guest
        // PRODUCED, never checked that playback consumed them at the right
        // rate, so a fully broken resampler passed them.
        this.base = 11025 / sampleRate;
        this.ratio = this.base;         // current (rate-controlled) step
        this.last = 0;                  // last emitted sample (click-free holds)
        this.lastReq = 0;               // hunger-request throttle
        this.starved = 0;               // output samples we could not fill
        this.produced = 0;              // samples received from the guest
        this.port.onmessage = (e) => {
            const s = e.data;           // transferred Float32Array
            this.produced += s.length;
            const consumed = Math.floor(this.pos);
            const keepFrom = consumed > TRIM_AT ? consumed : 0;
            const head = keepFrom ? this.q.subarray(keepFrom) : this.q;
            if (keepFrom) this.pos -= keepFrom;
            let merged = new Float32Array(head.length + s.length);
            merged.set(head, 0);
            merged.set(s, head.length);
            // Hard latency bound: if we somehow run far ahead (boot catch-up),
            // drop the OLDEST audio rather than growing unbounded.
            if (merged.length - this.pos > MAX_QUEUE) {
                const drop = Math.floor(merged.length - this.pos - MAX_QUEUE);
                merged = merged.subarray(drop);
                this.pos = Math.max(0, this.pos - drop);
            }
            this.q = merged;
        };
    }

    process(inputs, outputs) {
        const out = outputs[0];
        if (!out || !out[0]) return true;
        const ch = out[0];
        const n = ch.length;

        // ── rate control ───────────────────────────────────────────────────
        // Aim to hold TARGET_Q samples buffered.  Running the step
        // proportional to fill level makes the loop self-balancing: if the
        // guest delivers 63% of realtime, the rate settles at ~0.63 and the
        // buffer stops draining instead of gapping.
        const avail = this.q.length - this.pos;
        let desired = this.base * (avail / TARGET_Q);
        if (desired < this.base * RATIO_MIN) desired = this.base * RATIO_MIN;
        else if (desired > this.base * RATIO_MAX) desired = this.base * RATIO_MAX;
        this.ratio += (desired - this.ratio) * SMOOTH;

        for (let i = 0; i < n; i++) {
            const i0 = Math.floor(this.pos);
            if (i0 >= this.q.length - 1) {
                // True starvation: hold the last value and decay it to zero so
                // the gap is a soft fade, not a click.  Critically we do NOT
                // flush — everything already buffered stays and plays as soon
                // as the guest catches up.
                for (; i < n; i++) { this.last *= 0.995; ch[i] = this.last; this.starved++; }
                break;
            }
            const frac = this.pos - i0;
            this.last = this.q[i0] * (1 - frac) + this.q[i0 + 1] * frac;
            ch[i] = this.last;
            this.pos += this.ratio;
        }

        // Compact the consumed head occasionally (keeps `q` from growing).
        const consumed = Math.floor(this.pos);
        if (consumed > TRIM_AT) {
            this.q = this.q.slice(consumed);
            this.pos -= consumed;
        }

        // Hunger signal: ask the main thread to pump emulation steps whenever
        // the buffer is below target.  The audio thread keeps running even for
        // hidden/backgrounded tabs where rAF is paused — this is what keeps
        // audio alive in the background.
        if (avail < TARGET_Q && currentTime - this.lastReq > 0.02) {
            this.lastReq = currentTime;
            this.port.postMessage('need');
        }
        // Periodic telemetry for the smoke tests (starved samples = crackle).
        if (currentTime - (this.lastStat || 0) > 1) {
            this.lastStat = currentTime;
            this.port.postMessage({
                stat: true, starved: this.starved, produced: this.produced,
                avail, rate: this.ratio / this.base,
            });
        }
        return true;
    }
}

registerProcessor('doom-audio', DoomAudio);
