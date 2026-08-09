// In-browser network simulator: answers the eth_http firmware's DHCP, TCP
// and HTTP traffic with canned replies (no real stack, no gateway needed).
//
// Mirrors the openhw-local-gateway roles:
//   server MAC  = 5a:94:ef:e4:0c:dd  (the firmware's compile-time gw_mac)
//   server IP   = 192.168.4.1, offered client IP = 192.168.4.2
//   TCP target  = <client IP>:8092 (HTTP)

export function createNetSim({ log = () => {} } = {}) {
    const SERVER_MAC = [0x5a, 0x94, 0xef, 0xe4, 0x0c, 0xdd];
    const CLIENT_MAC = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    const SERVER_IP = [192, 168, 4, 1];
    const CLIENT_IP = [192, 168, 4, 2];
    const MASK = [255, 255, 255, 0];
    const HTTP_PORT = 8092;
    const HTTP_BODY =
        'HTTP/1.1 200 OK\r\n' +
        'Content-Length: 29\r\n' +
        'Connection: close\r\n\r\n' +
        'Hello from openhw HTTP server';

    let clientMac = CLIENT_MAC.slice();
    let srvSeq = 0x10000000; // server ISN
    let clientSeq = 0;       // learned from the client SYN
    let lastMsgType = 0;

    const stats = { tx: 0, rx: 0, dhcpOffers: 0, dhcpAcks: 0, synAcks: 0, httpResponses: 0 };

    const cksum = (bytes) => {
        let sum = 0;
        for (let i = 0; i < bytes.length; i += 2) {
            const w = (bytes[i] << 8) | (i + 1 < bytes.length ? bytes[i + 1] : 0);
            sum += w;
        }
        while (sum >> 16) sum = (sum & 0xffff) + (sum >> 16);
        return (~sum) & 0xffff;
    };

    // Build a complete Ethernet+IP(+UDP/TCP) frame with correct IP checksum.
    const buildFrame = (dstIp, proto, udpOrTcpPayload) => {
        const ipLen = 20 + udpOrTcpPayload.length;
        const f = new Uint8Array(14 + ipLen);
        f.set(dstIp === 255 ? [255,255,255,255,255,255] : clientMac, 0);
        f.set(SERVER_MAC, 6);
        f[12] = 0x08; f[13] = 0x00;
        const ip = 14;
        f[ip] = 0x45; f[ip + 1] = 0;
        f[ip + 2] = ipLen >> 8; f[ip + 3] = ipLen & 0xff;
        f[ip + 8] = 128; f[ip + 9] = proto;
        f.set(SERVER_IP, ip + 12);
        f.set(dstIp, ip + 16);
        const ck = cksum(f.subarray(ip, ip + 20));
        f[ip + 10] = ck >> 8; f[ip + 11] = ck & 0xff;
        f.set(udpOrTcpPayload, ip + 20);
        return f;
    };

    // ── DHCP reply (Offer or Ack) ──
    const dhcpReply = (req, msgType) => {
        // BOOTP fixed part
        const bootp = new Uint8Array(300);
        const xid = (req[4] << 24) | (req[5] << 16) | (req[6] << 8) | req[7];
        bootp[0] = 2;                          // op = reply
        bootp[1] = 1;                          // htype = ethernet
        bootp[2] = 6;                          // hlen
        bootp[4] = xid >> 24; bootp[5] = xid >> 16; bootp[6] = xid >> 8; bootp[7] = xid;
        bootp[8] = 0x80; bootp[9] = 0x00;      // broadcast flag
        bootp.set(CLIENT_IP, 16);              // yiaddr
        bootp.set(SERVER_IP, 20);              // siaddr
        // chaddr = client's chaddr (12 bytes: MAC + 6 pad)
        bootp.set(req.subarray(28, 40), 28);
        bootp.set([0x63, 0x82, 0x53, 0x63], 236); // magic cookie
        const opts = [53, 1, msgType,
                      1, 4, ...MASK,
                      3, 4, ...SERVER_IP,
                      6, 4, ...SERVER_IP,
                      54, 4, ...SERVER_IP,
                      51, 4, 0, 0, 1, 0x80,  // lease 86400
                      255];
        bootp.set(opts, 240);
        // UDP header (src 67, dst 68) + bootp
        const udp = new Uint8Array(8 + bootp.length);
        udp[0] = 0; udp[1] = 67; udp[2] = 0; udp[3] = 68;
        const ulen = 8 + bootp.length;
        udp[4] = ulen >> 8; udp[5] = ulen & 0xff;
        udp.set(bootp, 8);
        return buildFrame(255, 17, udp);       // dst IP = broadcast
    };

    // ── TCP ──
    const tcpFrame = (flags, seq, ack, payload) => {
        const data = payload || new Uint8Array(0);
        const tcp = new Uint8Array(20 + data.length);
        tcp[0] = HTTP_PORT >> 8; tcp[1] = HTTP_PORT & 0xff;   // src = server port
        tcp[2] = tcpSrcPort >> 8; tcp[3] = tcpSrcPort & 0xff; // dst = client's ephemeral
        tcp[4] = seq >> 24; tcp[5] = seq >> 16; tcp[6] = seq >> 8; tcp[7] = seq;
        tcp[8] = ack >> 24; tcp[9] = ack >> 16; tcp[10] = ack >> 8; tcp[11] = ack;
        tcp[12] = 0x50; tcp[13] = flags;
        tcp[14] = 0xff; tcp[15] = 0xff;
        tcp.set(data, 20);
        return buildFrame(CLIENT_IP, 6, tcp);
    };

    let tcpSrcPort = 0; // client ephemeral port, learned from SYN

    // Parse a TX frame; return an array of reply frames.
    function onTx(frame) {
        stats.tx++;
        const replies = [];
        if (frame.length < 14) return replies;
        const et = (frame[12] << 8) | frame[13];

        if (et === 0x1234) { // eth_irq_test: echo PING -> PONG
            if (new TextDecoder().decode(frame.subarray(14, 27)).includes('PING')) {
                const r = new Uint8Array(60);
                r.set(frame.subarray(6, 12), 0);  // dst = requester MAC
                r.set(frame.subarray(0, 6), 6);   // src = requester's dst MAC
                r[12] = 0x12; r[13] = 0x34;
                r.set(new TextEncoder().encode('ETH IRQ PONG'), 14);
                replies.push(r);
            }
            return replies;
        }

        if (et === 0x0806) { // ARP: answer requests for the server IP
            const a = 14;
            if (frame[a] === 0 && frame[a + 1] === 1 && frame[a + 6] === 0 && frame[a + 7] === 1) {
                const tip = [frame[a + 24], frame[a + 25], frame[a + 26], frame[a + 27]];
                if (tip[0] === SERVER_IP[0] && tip[1] === SERVER_IP[1] && tip[2] === SERVER_IP[2] && tip[3] === SERVER_IP[3]) {
                    const r = new Uint8Array(42);
                    r.set(frame.subarray(6, 12), 0);  // dst = requester MAC
                    r.set(SERVER_MAC, 6);             // src = our MAC
                    r[12] = 0x08; r[13] = 0x06;
                    const p = 14;
                    r[p] = 0; r[p + 1] = 1; r[p + 2] = 0x08; r[p + 3] = 0;
                    r[p + 4] = 6; r[p + 5] = 4;
                    r[p + 6] = 0; r[p + 7] = 2;        // reply
                    r.set(SERVER_MAC, p + 8);          // sha
                    r.set(SERVER_IP, p + 14);          // spa
                    r.set(frame.subarray(p + 8, p + 14), p + 18); // tha = requester
                    r.set(frame.subarray(p + 14, p + 18), p + 24); // tpa
                    replies.push(r);
                }
            }
            return replies;
        }

        if (et !== 0x0800) return replies;
        const proto = frame[23];
        const ihl = (frame[14] & 0x0f) * 4;
        const ipStart = 14 + ihl;

        if (proto === 17) { // UDP -> DHCP
            const sport = (frame[ipStart] << 8) | frame[ipStart + 1];
            const dport = (frame[ipStart + 2] << 8) | frame[ipStart + 3];
            if (sport === 68 && dport === 67) {
                const dhcp = frame.subarray(ipStart + 8);
                let mt = 0;
                const opt = dhcp.subarray(240);
                for (let o = 0; o < opt.length;) {
                    if (opt[o] === 0xff) break;
                    if (opt[o] === 0) { o++; continue; }
                    if (opt[o] === 53) { mt = opt[o + 2]; break; }
                    o += opt[o + 1] + 2;
                }
                clientMac = Array.from(dhcp.subarray(28, 34));
                lastMsgType = mt;
                if (mt === 1) { // Discover -> Offer
                    stats.dhcpOffers++;
                    log('DHCP Discover -> Offer (XID=0x' + ((dhcp[4] << 24) | (dhcp[5] << 16) | (dhcp[6] << 8) | dhcp[7]).toString(16).padStart(8, '0') + ')');
                    replies.push(dhcpReply(dhcp, 2));
                } else if (mt === 3) { // Request -> Ack
                    stats.dhcpAcks++;
                    log('DHCP Request -> Ack');
                    replies.push(dhcpReply(dhcp, 5));
                }
            }
            return replies;
        }

        if (proto === 6) { // TCP
            const sport = (frame[ipStart] << 8) | frame[ipStart + 1];
            const dport = (frame[ipStart + 2] << 8) | frame[ipStart + 3];
            if (sport !== 0 || dport !== HTTP_PORT) {
                // client -> server frames; learn the ephemeral port
                if (dport === HTTP_PORT) tcpSrcPort = sport;
                if (tcpSrcPort === 0) return replies;
            }
            const seq = (frame[ipStart + 4] << 24) | (frame[ipStart + 5] << 16) | (frame[ipStart + 6] << 8) | frame[ipStart + 7];
            const fl = frame[ipStart + 13];
            const th = ((frame[ipStart + 12] >> 4) & 0x0f) * 4;
            const dlen = frame.length - ipStart - th;

            if (fl === 0x02) { // SYN
                clientSeq = seq;
                stats.synAcks++;
                log('TCP SYN -> SYN-ACK (seq=' + seq + ')');
                replies.push(tcpFrame(0x12, srvSeq, seq + 1, null));
            } else if ((fl & 0x18) === 0x18 && dlen > 0) { // PSH|ACK with data (HTTP GET)
                const ack = seq + dlen;
                log('HTTP GET (' + dlen + 'B) -> ACK + 200 response');
                replies.push(tcpFrame(0x10, srvSeq, ack, null));
                replies.push(tcpFrame(0x19, srvSeq, ack, new TextEncoder().encode(HTTP_BODY))); // PSH|ACK|FIN
                stats.httpResponses++;
            }
            return replies;
        }
        return replies;
    }

    return { onTx, stats };
}
