// AudioWorklet for the DOOM page: resamples the guest's 11025 Hz mono
// stream to the context sample rate and plays it continuously on the audio
// thread.  No BufferSource scheduling jitter; the queue is bounded (oldest
// samples dropped when production runs ahead of realtime, e.g. the one-time
// boot catch-up burst), so latency never exceeds MAX_QUEUE.
const MAX_QUEUE = 4096;                 // ~0.37 s at 11025 Hz

class DoomAudio extends AudioWorkletProcessor {
    constructor() {
        super();
        this.q = new Float32Array(0);   // pending 11025 Hz samples
        this.pos = 0;                   // read cursor in 11025 Hz space
        this.ratio = sampleRate / 11025;
        this.lastReq = 0;               // last hunger-request time
        this.port.onmessage = (e) => {
            const s = e.data;           // transferred Float32Array
            const n = this.q.length + s.length;
            if (n > MAX_QUEUE) {
                const drop = n - MAX_QUEUE;          // drop oldest
                const qKeep = Math.max(0, this.q.length - drop);
                const sKeep = MAX_QUEUE - qKeep;
                const merged = new Float32Array(MAX_QUEUE);
                merged.set(this.q.subarray(Math.min(drop, this.q.length)), 0);
                merged.set(s.subarray(0, sKeep), qKeep);
                this.q = merged;
                return;
            }
            const merged = new Float32Array(n);
            merged.set(this.q, 0);
            merged.set(s, this.q.length);
            this.q = merged;
        };
    }
    process(inputs, outputs) {
        const out = outputs[0];
        if (!out || !out[0]) return true;
        const ch = out[0];
        const n = ch.length;
        for (let i = 0; i < n; i++) {
            const i0 = Math.floor(this.pos);
            if (i0 >= this.q.length - 1) {
                // underrun (production stalled, e.g. throttled rAF): emit
                // silence and DROP the stale queue so old samples never
                // replay after a gap; the next postMessage plays immediately.
                for (; i < n; i++) ch[i] = 0;
                this.q = new Float32Array(0);
                this.pos = 0;
                break;
            }
            const frac = this.pos - i0;
            ch[i] = this.q[i0] * (1 - frac) + this.q[i0 + 1] * frac;
            this.pos += this.ratio;
        }
        if (this.pos > this.q.length) {
            this.q = this.q.subarray(Math.floor(this.pos));
            this.pos = 0;
        }
        // Hunger signal: ask the main thread to pump emulation steps whenever
        // the queue is below max (production should keep the queue topped up
        // at all times; the throttle prevents message floods).  The audio
        // thread (and its port messages) keeps running even for hidden/
        // backgrounded tabs where rAF is paused — this is what keeps audio
        // lockstep in the background.
        if (this.q.length < MAX_QUEUE && currentTime - this.lastReq > 0.02) {
            this.lastReq = currentTime;
            this.port.postMessage('need');
        }
        return true;
    }
}

registerProcessor('doom-audio', DoomAudio);
