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
    // The dst MAC for unsolicited server frames (FRAG/LLDP/IGMP...).
    // DHCP learns the real chaddr, but eth_adv never runs DHCP — seed
    // from any TX frame's source MAC so peers address the guest.
    const learnMac = (frame) => {
        if (frame.length >= 12) {
            const sa = Array.from(frame.subarray(6, 12));
            if (sa.some((b) => b !== 0)) clientMac = sa;
        }
    };
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
    let echoSport = 0;  // TCP-echo (port 7) client port, learned from SYN
    let echoSeq = 0x20000000; // TCP-echo server ISN
    // TCP-server-test client role (lwip_demo listens on 7): fixed sport,
    // connects when the firmware sends a UDP trigger to port 5004.
    let cliSport = 5005, cliSeq = 0x30000000, cliSrvIss = 0, cliFwIp = null;
    let igmpReports = 0; // eth_adv IGMP phase: 1st report -> query, 2nd -> + traffic
    // eth_adv peers (dedicated server ports 5010-5018): per-port server
    // ISN + per-port noticed MSS (the guest SYN option is parsed at SYN
    // time so the SYN-ACK can echo the clamped value back).
    let advIss = 0x40000000, advMss = 1460;
    const ADV_PORTS = new Set([5010, 5011, 5012, 5013]);
    const DNS_IP = [93, 184, 216, 34]; // canned A answer (example.com real IP)

    // TCP segment builder with explicit ports (echo path; HTTP uses tcpFrame).
    const tcpSeg = (sport, dport, flags, seq, ack, payload, dstIp) => {
        const data = payload || new Uint8Array(0);
        const tcp = new Uint8Array(20 + data.length);
        tcp[0] = sport >> 8; tcp[1] = sport & 0xff;
        tcp[2] = dport >> 8; tcp[3] = dport & 0xff;
        tcp[4] = seq >> 24; tcp[5] = seq >> 16; tcp[6] = seq >> 8; tcp[7] = seq;
        tcp[8] = ack >> 24; tcp[9] = ack >> 16; tcp[10] = ack >> 8; tcp[11] = ack;
        tcp[12] = 0x50; tcp[13] = flags;
        tcp[14] = 0xff; tcp[15] = 0xff;
        tcp.set(data, 20);
        // TCP checksum over pseudo-header.
        const srcIp = SERVER_IP;
        let sum = 0;
        const add = (b, o, n) => { for (let i = 0; i < n; i += 2) sum += (b[o + i] << 8) | (i + 1 < n ? b[o + i + 1] : 0); };
        add(srcIp, 0, 4); add(dstIp, 0, 4);
        sum += 6 + tcp.length;
        add(tcp, 0, tcp.length);
        while (sum >> 16) sum = (sum & 0xffff) + (sum >> 16);
        const ck = (~sum) & 0xffff;
        tcp[16] = ck >> 8; tcp[17] = ck & 0xff;
        return buildFrame(dstIp, 6, tcp);
    };

    // Parse a TX frame; return an array of reply frames.
    function onTx(frame) {
        stats.tx++;
        const replies = [];
        if (frame.length < 14) return replies;
        learnMac(frame);
        // FEAT loopback-silence: the feat firmware's loopback phases
        // address their own MAC (02:00:00:00:00:01, SARC or zero SA)
        // with UDP ports 5009+ and expect point-to-point silence — no
        // peer replies (a canned answer would land in the guest's RX
        // window and poison the silence probes). Multicast
        // (01:00:5E:...), broadcast, pause, and the server-MAC
        // DHCP/DNS/echo peer traffic still answer below. (Checked
        // FIRST, before ethertype dispatch — the 0x1234 PING branch
        // below would otherwise answer feat-shaped frames.)
        if (frame.length >= 14 && frame[12] === 0x08 && (frame[13] === 0x00 || frame[13] === 0x06)) {
            const mcast = (frame[0] & 1) !== 0;
            const toSelf = !mcast && frame[0] === 0x02 && frame[1] === 0x00 && frame[2] === 0x00 &&
                frame[3] === 0x00 && frame[4] === 0x00 && frame[5] === 0x01;
            if (toSelf && frame.length >= 42) {
                const dport = (frame[36] << 8) | frame[37];
                if (dport >= 5009) return replies;
            }
        }
        // Silently ignore flow-control pause frames (multicast pause DA,
        // 0x8808/0001): they are link-level, never answered (a canned
        // reply here would land in the guest's RX window and poison
        // "expect silence" probes like PAUSE TERM).
        if (frame.length >= 18 && frame[0] === 0x01 && frame[1] === 0x80 &&
            frame[2] === 0xC2 && frame[3] === 0x00 && frame[4] === 0x00 && frame[5] === 0x01 &&
            frame[12] === 0x88 && frame[13] === 0x08 && frame[14] === 0x00 && frame[15] === 0x01)
            return replies;
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

        if (et === 0x86dd) return replies; // IPv6: never answered (no v6 stack)
        if (et === 0x88cc) return replies; // LLDP: never answered
        if (frame.length >= 17 && frame[14] === 0x42 && frame[15] === 0x42) return replies; // STP LLC: never answered

        if (et === 0x0806) { // ARP: answer requests (any target IP — canned sim
            // claims the requested address with SERVER_MAC so guest ARP never stalls)
            const a = 14;
            if (frame[a] === 0 && frame[a + 1] === 1 && frame[a + 6] === 0 && frame[a + 7] === 1) {
                const r = new Uint8Array(42);
                r.set(frame.subarray(6, 12), 0);  // dst = requester MAC
                r.set(SERVER_MAC, 6);             // src = our MAC
                r[12] = 0x08; r[13] = 0x06;
                const p = 14;
                r[p] = 0; r[p + 1] = 1; r[p + 2] = 0x08; r[p + 3] = 0;
                r[p + 4] = 6; r[p + 5] = 4;
                r[p + 6] = 0; r[p + 7] = 2;        // reply
                r.set(SERVER_MAC, p + 8);          // sha
                r.set(frame.subarray(a + 24, a + 28), p + 14); // spa = requested IP
                r.set(frame.subarray(p + 8, p + 14), p + 18); // tha = requester
                r.set(frame.subarray(p + 14, p + 18), p + 24); // tpa
                replies.push(r);
            }
            return replies;
        }

        if (et !== 0x0800) return replies;
        // (Feat loopback-silence is handled by the DA check at the top:
        // self-addressed frames never reach the UDP/TCP/ICMP answers.)
        const proto = frame[23];
        const ihl = (frame[14] & 0x0f) * 4;
        const ipStart = 14 + ihl;

        if (proto === 2) { // IGMP: report -> query (+ group traffic on 2nd)
            const type = frame[ipStart];
            const grp = [frame[ipStart + 4], frame[ipStart + 5], frame[ipStart + 6], frame[ipStart + 7]];
            igmpReports++;
            if (type === 0x16 && grp[0] === 239 && grp[1] === 0 && grp[2] === 0 && grp[3] === 9) {
                // General query (dst 224.0.0.1, group 0.0.0.0).
                const q = new Uint8Array(42);
                q.set([0x01, 0x00, 0x5E, 0x00, 0x00, 0x01], 0); q.set(SERVER_MAC, 6);
                q[12] = 0x08; q[13] = 0x00;
                q[14] = 0x45; q[17] = 28; q[22] = 1; q[23] = 2;
                q.set(SERVER_IP, 26); q.set([224, 0, 0, 1], 30);
                let s = 0; for (let i = 14; i < 34; i += 2) s += (q[i] << 8) | q[i + 1];
                while (s >> 16) s = (s & 0xffff) + (s >> 16); s = (~s) & 0xffff;
                q[24] = s >> 8; q[25] = s & 0xff;
                q[34] = 0x11; q[35] = 100; // query, max-resp 10s
                q[36] = 0; q[37] = 0;
                {
                    let c = 0;
                    for (let i = 34; i < 42; i += 2) c += (q[i] << 8) | q[i + 1];
                    while (c >> 16) c = (c & 0xffff) + (c >> 16); c = (~c) & 0xffff;
                    q[36] = c >> 8; q[37] = c & 0xff;
                }
                replies.push(q);
                log('IGMP report -> query');
                if (igmpReports >= 2) { // joined: send the group traffic
                    const p = new TextEncoder().encode('GROUP9');
                    const u = new Uint8Array(8 + p.length);
                    u[0] = 0x13; u[1] = 0x98; u[2] = 0xC0; u[3] = 0x02;
                    u[4] = (8 + p.length) >> 8; u[5] = (8 + p.length) & 0xff;
                    u.set(p, 8);
                    const ipLen = 20 + u.length;
                    const fr = new Uint8Array(14 + ipLen);
                    fr.set([0x01, 0x00, 0x5E, 0x00, 0x00, 0x09], 0); fr.set(SERVER_MAC, 6);
                    fr[12] = 0x08; fr[13] = 0x00;
                    fr[14] = 0x45; fr[16] = ipLen >> 8; fr[17] = ipLen & 0xff;
                    fr[22] = 1; fr[23] = 17;
                    fr.set(SERVER_IP, 26); fr.set([239, 0, 0, 9], 30);
                    const ck = cksum(fr.subarray(14, 34));
                    fr[24] = ck >> 8; fr[25] = ck & 0xff;
                    fr.set(u, 34);
                    replies.push(fr);
                    log('IGMP group traffic');
                }
            }
            return replies;
        }

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
                } else if (mt === 3) { // Request -> Ack (or NAK for the unknown xid)
                    const xid = ((dhcp[4] << 24) | (dhcp[5] << 16) | (dhcp[6] << 8) | dhcp[7]) >>> 0;
                    if (xid === 0xDEADBEEF) { // eth_adv NAK trigger: refuse it
                        stats.dhcpNaks = (stats.dhcpNaks || 0) + 1;
                        log('DHCP Request -> NAK');
                        replies.push(dhcpReply(dhcp, 6));
                    } else {
                        stats.dhcpAcks++;
                        log('DHCP Request -> Ack');
                        replies.push(dhcpReply(dhcp, 5));
                    }
                }
            } else if (dport === 53) { // DNS: canned A answer (any query)
                // q = DNS message (ipStart already points past the IP
                // header, +8 more skips the UDP header).
                const q = frame.subarray(ipStart + 8);
                const txid = (q[0] << 8) | q[1];
                let qend = 12; // question labels live past the DNS header
                while (qend < q.length && q[qend] !== 0) qend += q[qend] + 1;
                // Question length RELATIVE to its start (name + NUL +
                // QTYPE + QCLASS) — not including the DNS header, or the
                // echo duplicates the header and parsers run off the rails.
                const qlen = (qend - 12) + 5;
                const resp = new Uint8Array(12 + qlen + 16);
                resp[0] = txid >> 8; resp[1] = txid & 0xff;
                resp[2] = 0x81; resp[3] = 0x80; // response, no error
                resp[4] = 0; resp[5] = 1; resp[6] = 0; resp[7] = 1;
                resp.set(q.subarray(12, 12 + qlen), 12);
                let o = 12 + qlen;
                resp[o] = 0xc0; resp[o + 1] = 0x0c; // NAME ptr -> question
                resp[o + 2] = 0; resp[o + 3] = 1;   // TYPE A
                resp[o + 4] = 0; resp[o + 5] = 1;   // CLASS IN
                resp[o + 6] = 0; resp[o + 7] = 0; resp[o + 8] = 0; resp[o + 9] = 60;
                resp[o + 10] = 0; resp[o + 11] = 4;
                resp.set(DNS_IP, o + 12);
                const udp = new Uint8Array(8 + resp.length);
                udp[0] = 0; udp[1] = 53;
                udp[2] = frame[ipStart]; udp[3] = frame[ipStart + 1]; // dst = query src port
                const ulen = 8 + resp.length;
                udp[4] = ulen >> 8; udp[5] = ulen & 0xff;
                udp.set(resp, 8);
                // src IP = the query's IP source (ipStart-8), NOT ipStart+12
                // (that's inside the DNS message past the UDP header).
                const srcIp = [frame[ipStart - 8], frame[ipStart - 7], frame[ipStart - 6], frame[ipStart - 5]];
                const f = buildFrame(srcIp, 17, udp);
                f.set(frame.subarray(6, 12), 0);
                f.set(SERVER_MAC, 6);
                stats.dnsAnswers = (stats.dnsAnswers || 0) + 1;
                log('DNS query -> A ' + DNS_IP.join('.'));
                replies.push(f);
            } else if (dport === 7) { // UDP echo: swap ports, return payload
                // (Feat silence-range already returned above when either
                // port is 5009+ — this answers only genuine port-7 peer
                // traffic like eth_test/lwip_demo.)
                const udpLen = (frame[ipStart + 4] << 8) | frame[ipStart + 5];
                const payload = frame.subarray(ipStart + 8, ipStart + udpLen);
                const udp = new Uint8Array(8 + payload.length);
                udp[0] = 0; udp[1] = 7;
                udp[2] = frame[ipStart]; udp[3] = frame[ipStart + 1];
                const ulen = 8 + payload.length;
                udp[4] = ulen >> 8; udp[5] = ulen & 0xff;
                udp.set(payload, 8);
                const srcIp = [frame[ipStart - 8], frame[ipStart - 7], frame[ipStart - 6], frame[ipStart - 5]];
                const f = buildFrame(srcIp, 17, udp);
                f.set(frame.subarray(6, 12), 0);
                f.set(SERVER_MAC, 6);
                stats.udpEchoes = (stats.udpEchoes || 0) + 1;
                log('UDP echo (' + payload.length + 'B)');
                replies.push(f);
            } else if (dport === 5001) { // eth_feat_test: multicast pair
                // Member frame (group 239.0.0.7) + non-member (239.0.0.8).
                // The firmware joins only the first via the hash table.
                const mk = (mac, ip, text) => {
                    const p = new TextEncoder().encode(text);
                    const u = new Uint8Array(8 + p.length);
                    u[0] = 0x13; u[1] = 0x89; // sport 5001
                    u[2] = 0x13; u[3] = 0x8A; // dport 5002
                    u[4] = (8 + p.length) >> 8; u[5] = (8 + p.length) & 0xff;
                    u.set(p, 8);
                    const ipLen = 20 + u.length;
                    const fr = new Uint8Array(14 + ipLen);
                    fr.set(mac, 0); fr.set(SERVER_MAC, 6);
                    fr[12] = 0x08; fr[13] = 0x00;
                    fr[14] = 0x45;
                    fr[16] = ipLen >> 8; fr[17] = ipLen & 0xff;
                    fr[22] = 128; fr[23] = 17;
                    fr.set(SERVER_IP, 26); fr.set(ip, 30);
                    const ck = cksum(fr.subarray(14, 34));
                    fr[24] = ck >> 8; fr[25] = ck & 0xff;
                    fr.set(u, 34);
                    return fr;
                };
                replies.push(mk([0x01, 0x00, 0x5E, 0x00, 0x00, 0x07], [239, 0, 0, 7], 'MCAST1'));
                replies.push(mk([0x01, 0x00, 0x5E, 0x00, 0x00, 0x08], [239, 0, 0, 8], 'MCAST2'));
                log('multicast member + non-member');
            } else if (dport === 5002) { // eth_feat_test: VLAN pair
                // Tagged VID 7 (accepted when VLANTI=7) + untagged twin.
                const mkVlan = (tag, text) => {
                    const p = new TextEncoder().encode(text);
                    const u = new Uint8Array(8 + p.length);
                    u[0] = 0x13; u[1] = 0x8A; u[2] = 0x13; u[3] = 0x8B;
                    u[4] = (8 + p.length) >> 8; u[5] = (8 + p.length) & 0xff;
                    u.set(p, 8);
                    const ipLen = 20 + u.length;
                    const fr = new Uint8Array(14 + (tag ? 4 : 0) + ipLen);
                    fr.set(clientMac, 0); fr.set(SERVER_MAC, 6);
                    let l3 = 14;
                    if (tag) {
                        fr[12] = 0x81; fr[13] = 0x00; fr[14] = 0x00; fr[15] = 0x07;
                        l3 = 18;
                    } else { fr[12] = 0x08; fr[13] = 0x00; }
                    fr[l3] = 0x45;
                    fr[l3 + 2] = ipLen >> 8; fr[l3 + 3] = ipLen & 0xff;
                    fr[l3 + 8] = 128; fr[l3 + 9] = 17;
                    fr.set(SERVER_IP, l3 + 12); fr.set(CLIENT_IP, l3 + 16);
                    const ck = cksum(fr.subarray(l3, l3 + 20));
                    fr[l3 + 10] = ck >> 8; fr[l3 + 11] = ck & 0xff;
                    fr.set(u, l3 + 20);
                    return fr;
                };
                replies.push(mkVlan(true, 'VLAN7'));
                replies.push(mkVlan(false, 'NOVLAN'));
                log('VLAN tagged + untagged');
            } else if (dport === 5003) { // eth_feat_test: magic packet
                const mp = new Uint8Array(14 + 102);
                mp.set(clientMac, 0); mp.set(SERVER_MAC, 6);
                mp[12] = 0x08; mp[13] = 0x00;
                mp.set([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 14);
                for (let r = 0; r < 16; r++) mp.set(clientMac, 20 + r * 6);
                replies.push(mp);
                log('magic packet');
            } else if (dport === 5004) { // lwip_demo: TCP client connects to :7
                // Firmware listens on port 7; open from sport 5005.
                cliFwIp = [frame[ipStart - 8], frame[ipStart - 7], frame[ipStart - 6], frame[ipStart - 5]];
                log('TCP client SYN -> :7');
                replies.push(tcpSeg(cliSport, 7, 0x02, cliSeq, 0, null, cliFwIp));
            } else if (dport >= 5010 && dport <= 5018) { // eth_adv triggers
                // One UDP probe to a dedicated server port drives one
                // canned peer. The probe's sport is echoed where a reply
                // port matters (ICMP quote, TCP ports below).
                const advSport = (frame[ipStart] << 8) | frame[ipStart + 1];
                const advSrcIp = [frame[ipStart - 8], frame[ipStart - 7], frame[ipStart - 6], frame[ipStart - 5]];
                const advUdpLen = (frame[ipStart + 4] << 8) | frame[ipStart + 5];
                const advUl = (o, n) => { const u = new Uint8Array(8 + n); u[0] = o >> 8; u[1] = o & 0xff; u[2] = advSport >> 8; u[3] = advSport & 0xff; u[4] = (8 + n) >> 8; u[5] = (8 + n) & 0xff; return u; };
                if (dport === 5014) { // FRAG: two IP fragments, "FRAGMENT!"
                    // Real RFC 791 fragmentation of one UDP datagram:
                    // datagram payload = UDP header (8) + "FRAGMENT!" (9).
                    // frag0 carries bytes [0..16) (UDP hdr + "FRAGMENT"),
                    // off=0 MF=1; frag1 carries bytes [16..17) ("!"),
                    // off=2 (16 bytes) MF=0. Only frag0 has the UDP
                    // header — frag1 is a raw payload continuation.
                    const ident = 0x1234;
                    const full = (() => {
                        const u = new Uint8Array(8 + 9);
                        u[0] = 0x13; u[1] = 0x96; u[2] = advSport >> 8; u[3] = advSport & 0xff;
                        u[4] = 0; u[5] = 17;
                        u.set(new TextEncoder().encode('FRAGMENT!'), 8);
                        return u;
                    })();
                    const mkfrag = (off8, mf, slice) => {
                        const ipLen = 20 + slice.length;
                        const fr = new Uint8Array(14 + ipLen);
                        // dst = guest MAC (unicast to us, like every other
                        // server peer — the model has no promiscuous bit
                        // set, so server-MAC sources never deliver).
                        fr.set(clientMac, 0); fr.set(SERVER_MAC, 6);
                        fr[12] = 0x08; fr[13] = 0x00;
                        fr[14] = 0x45; fr[16] = ipLen >> 8; fr[17] = ipLen & 0xff;
                        fr[18] = ident >> 8; fr[19] = ident & 0xff;
                        // FLAGS+FRAGOFF (RFC 791 §3.1): bit 13 (0x2000) =
                        // MF, low 13 bits = offset in 8-byte units. The
                        // field is big-endian: high byte carries MF+off[12:8].
                        fr[20] = (mf ? 0x20 : 0) | ((off8 >> 8) & 0x1F); fr[21] = off8 & 0xff;
                        fr[22] = 64; fr[23] = 17;
                        fr.set(SERVER_IP, 26); fr.set(advSrcIp, 30);
                        const ck = cksum(fr.subarray(14, 34));
                        fr[24] = ck >> 8; fr[25] = ck & 0xff;
                        fr.set(slice, 34);
                        return fr;
                    };
                    replies.push(mkfrag(0, true, full.subarray(0, 16)));
                    replies.push(mkfrag(2, false, full.subarray(16, 17)));
                    log('FRAG 2 fragments');
                } else if (dport === 5015) { // ICMPERR: port-unreachable quoting the probe
                    // Quote the full inner IP datagram (RFC 792: IP header
                    // + 8 bytes of payload = 28 bytes here). The guest pads
                    // short triggers to the 60 B wire minimum, but the IP
                    // total-length field says 32 — quote EXACTLY that many
                    // bytes (innerTotal), so a strict checker comparing the
                    // quoted UDP length against the real length passes.
                    // (Quoting frame.length-ipStart would append 28 bytes
                    // of zero pad and the quoted UDP length reads 40.)
                    // L3 HEADER OFFSET (not L4): ipStart points at the UDP
                    // header (14+ihl); the IP header starts at 14. The old
                    // code read innerTotal off the UDP sport bytes (C0 01 =
                    // 49153) and quoted from the UDP header — so the reply
                    // carried C0 01... where the guest expects 45 00....
                    const l3 = ipStart - ihl;
                    const innerTotal = ((frame[l3 + 2] << 8) | frame[l3 + 3]);
                    const qlen = Math.min(28, innerTotal);
                    const ic = new Uint8Array(8 + qlen);
                    ic[0] = 3; ic[1] = 3;
                    ic.set(frame.subarray(l3, l3 + qlen), 8);
                    ic[2] = 0; ic[3] = 0;
                    const ck = cksum(ic);
                    ic[2] = ck >> 8; ic[3] = ck & 0xff;
                    const f = buildFrame(advSrcIp, 1, ic);
                    f.set(frame.subarray(6, 12), 0); f.set(SERVER_MAC, 6);
                    replies.push(f);
                    log('ICMP port-unreachable');
                } else if (dport === 5016) { // ND: IPv6 NS for our link-local
                    const ns = new Uint8Array(92);
                    ns.set(frame.subarray(6, 12), 0); ns.set(SERVER_MAC, 6);
                    ns[12] = 0x86; ns[13] = 0xDD;
                    ns[14] = 0x60; // version 6
                    ns[18] = 0; ns[19] = 32; // payload len 32
                    ns[20] = 58; ns[21] = 64; // ICMPv6, hop 64
                    ns.set([0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1], 22); // dst ll
                    ns.set([0xFE, 0x80, 0, 0, 0, 0, 0, 0, 0x5A, 0x94, 0xFF, 0xFE, 0xE4, 0x0C, 0xDD, 0x02], 38); // src ll
                    ns[54] = 135; ns[55] = 0; // NS
                    ns[58] = 0; ns[59] = 0;
                    ns.set([0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xFF, 0x0C, 0xDD, 0x02], 62); // target = solicited node
                    ns[78] = 1; ns[79] = 1; ns.set(SERVER_MAC, 80); // src lladdr option
                    replies.push(ns);
                    log('IPv6 NS');
                } else if (dport === 5017) { // LLDP: chassis MAC TLV
                    const ll = new Uint8Array(14 + 9 + 9);
                    ll.set(frame.subarray(6, 12), 0);
                    ll[0] = 0x01; ll[1] = 0x80; ll[2] = 0xC2; ll[3] = 0x00; ll[4] = 0x00; ll[5] = 0x0E;
                    ll.set(SERVER_MAC, 6);
                    ll[12] = 0x88; ll[13] = 0xCC;
                    ll[14] = 0x02; ll[15] = 0x07; ll[16] = 0x04; ll.set(SERVER_MAC, 17); // chassis MAC
                    ll[23] = 0x04; ll[24] = 0x04; ll[25] = 0x05; ll.set([0x65, 0x74, 0x68, 0x30], 26); // port "eth0"
                    ll[30] = 0x00; ll[31] = 0x00; // end TLV
                    replies.push(ll);
                    log('LLDP chassis');
                } else if (dport === 5018) { // STP: config BPDU, root 8000+srv MAC
                    // 802.3+LLC framing (no ethertype): dst(6) src(6)
                    // len(2) DSAP/SSAP/CTL(3) then the BPDU. No pad byte —
                    // an earlier revision inserted one at +17 and shifted
                    // every BPDU field by one (guest read flags as type).
                    const st = new Uint8Array(60);
                    st.set([0x01, 0x80, 0xC2, 0x00, 0x00, 0x00], 0);
                    st.set(SERVER_MAC, 6);
                    st[12] = 0x00; st[13] = 0x26; // length 38
                    st[14] = 0x42; st[15] = 0x42; st[16] = 0x03; // LLC
                    st[17] = 0x00; st[18] = 0x00; st[19] = 0x00; // proto/version/type=config
                    st[20] = 0x00; // flags
                    st[21] = 0x80; st[22] = 0x00; st.set(SERVER_MAC, 23); // root id
                    st[29] = 0; st[30] = 0; // root path cost
                    st[31] = 0x80; st[32] = 0x00; st.set(SERVER_MAC, 33); // bridge id
                    st[39] = 0x80; st[40] = 0x01; // port id
                    st[43] = 0x01; st[44] = 0x00; // hello time
                    replies.push(st);
                    log('STP config BPDU');
                } else if (dport >= 5010 && dport <= 5013) {
                    // TCP phases handled below (proto 6 branch owns the
                    // handshake); UDP probes here are ignored.
                } else {
                    void advUl; void advUdpLen;
                }
            }
            return replies;
        }

        if (proto === 1) { // ICMP: echo request -> echo reply (any dst IP)
            const type = frame[ipStart];
            // (Feat loopback-silence already returned above for
            // self-DA frames; what reaches here is genuine peer
            // traffic like eth_test's ping.)
            if (type === 8) {
                const icmpLen = frame.length - ipStart;
                const rep = new Uint8Array(icmpLen);
                rep.set(frame.subarray(ipStart, ipStart + icmpLen));
                rep[0] = 0; rep[1] = 0; // type/code = echo reply
                rep[2] = 0; rep[3] = 0;
                const ck = cksum(rep);
                rep[2] = ck >> 8; rep[3] = ck & 0xff;
                const srcIp = [frame[ipStart - 8], frame[ipStart - 7], frame[ipStart - 6], frame[ipStart - 5]];
                const f = buildFrame(srcIp, 1, rep);
                f.set(frame.subarray(6, 12), 0);
                f.set(SERVER_MAC, 6);
                // Be chatty back: also ping the firmware so its RX path
                // sees an echo request (proves reply handling both ways).
                const ping = new Uint8Array(8 + 4);
                ping[0] = 8; ping[1] = 0;
                ping[4] = 0x12; ping[5] = 0x34; ping[6] = 0; ping[7] = 0x01;
                ping[8] = 0xAA; ping[9] = 0xBB; ping[10] = 0xCC; ping[11] = 0xDD;
                const pck = cksum(ping);
                ping[2] = pck >> 8; ping[3] = pck & 0xff;
                const pf = buildFrame(CLIENT_IP, 1, ping);
                pf.set(clientMac, 0); pf.set(SERVER_MAC, 6);
                stats.icmpReplies = (stats.icmpReplies || 0) + 1;
                log('ICMP echo -> reply + return ping');
                replies.push(f, pf);
            }
            return replies;
        }

        if (proto === 6) { // TCP
            const sport = (frame[ipStart] << 8) | frame[ipStart + 1];
            const dport = (frame[ipStart + 2] << 8) | frame[ipStart + 3];
            const srcIp = [frame[ipStart - 8], frame[ipStart - 7], frame[ipStart - 6], frame[ipStart - 5]];
            if (ADV_PORTS.has(dport)) { // eth_adv: one canned peer per port
                const seq = (frame[ipStart + 4] << 24) | (frame[ipStart + 5] << 16) | (frame[ipStart + 6] << 8) | frame[ipStart + 7];
                const fl = frame[ipStart + 13];
                const th = ((frame[ipStart + 12] >> 4) & 0x0f) * 4;
                // SYN option parse: remember the guest's MSS so the
                // SYN-ACK echoes the clamped value (MSS phase).
                const mssOf = () => {
                    let m = 1460;
                    for (let k = 20; k + 3 < th;) {
                        const kind = frame[ipStart + k];
                        if (kind === 0) break;
                        if (kind === 1) { k++; continue; }
                        const kl = frame[ipStart + k + 1];
                        if (kind === 2 && kl === 4) { m = (frame[ipStart + k + 2] << 8) | frame[ipStart + k + 3]; break; }
                        if (kl < 2) break;
                        k += kl;
                    }
                    return m;
                };
                // SYN-ACK with the MSS option + a fixed small window.
                // (WINDOW phase reads win=1000 off any server ACK; the
                // same value rides here so it holds for the whole flow.)
                const advSynAck = (ackn) => {
                    const tcp = new Uint8Array(24);
                    tcp[0] = dport >> 8; tcp[1] = dport & 0xff;
                    tcp[2] = sport >> 8; tcp[3] = sport & 0xff;
                    tcp[4] = advIss >> 24; tcp[5] = advIss >> 16; tcp[6] = advIss >> 8; tcp[7] = advIss & 0xff;
                    tcp[8] = ackn >> 24; tcp[9] = ackn >> 16; tcp[10] = ackn >> 8; tcp[11] = ackn & 0xff;
                    tcp[12] = 0x60; tcp[13] = 0x12;
                    tcp[14] = 1000 >> 8; tcp[15] = 1000 & 0xff; // window
                    tcp[20] = 2; tcp[21] = 4; tcp[22] = advMss >> 8; tcp[23] = advMss & 0xff;
                    let sum = 0;
                    const add = (b, o, n) => { for (let i = 0; i < n; i += 2) sum += (b[o + i] << 8) | (i + 1 < n ? b[o + i + 1] : 0); };
                    add(SERVER_IP, 0, 4); add(srcIp, 0, 4);
                    sum += 6 + tcp.length;
                    add(tcp, 0, tcp.length);
                    while (sum >> 16) sum = (sum & 0xffff) + (sum >> 16);
                    const ck = (~sum) & 0xffff;
                    tcp[16] = ck >> 8; tcp[17] = ck & 0xff;
                    return buildFrame(srcIp, 6, tcp);
                };
                if (fl === 0x02) { // SYN
                    if (dport === 5010) { // RST: abort, no handshake
                        log('ADV RST');
                        replies.push(tcpSeg(dport, sport, 0x14, 0, seq + 1, null, srcIp));
                        return replies;
                    }
                    advMss = mssOf();
                    log('ADV SYN -> SYN-ACK (mss=' + advMss + ')');
                    replies.push(advSynAck(seq + 1));
                    advIss = (advIss + 1) >>> 0; // SYN consumes one sequence number
                    return replies;
                }
                // RTO (5011): blackhole — answer nothing, the guest's
                // wait loop must expire on its own.
                if (dport === 5011) return replies;
                // Post-handshake ACKs carry window 1000 (WINDOW phase
                // reads it off any server ACK after its 1-byte probe).
                if ((fl & 0x10) && dport >= 5012) {
                    const ipTot = (frame[16] << 8) | frame[17];
                    const dlen = Math.max(0, Math.min(frame.length - ipStart - th, ipTot - (ipStart - 14) - th));
                    const ackn = (seq + dlen) >>> 0;
                    log('ADV ACK win=1000');
                    // tcpSeg hardcodes window 0xffff — patch the window
                    // bytes (L4+14) then repair the TCP checksum in place
                    // (the IP header covers only itself, untouched).
                    const f = tcpSeg(dport, sport, 0x10, advIss, ackn, null, srcIp);
                    const l4 = f.length - 20;
                    f[l4 + 14] = 1000 >> 8; f[l4 + 15] = 1000 & 0xff;
                    f[l4 + 16] = 0; f[l4 + 17] = 0;
                    let s2 = 0;
                    const a2 = (b, o, n) => { for (let i = 0; i < n; i += 2) s2 += (b[o + i] << 8) | (i + 1 < n ? b[o + i + 1] : 0); };
                    a2(SERVER_IP, 0, 4); a2(srcIp, 0, 4);
                    s2 += 6 + 20;
                    a2(f, l4, 20);
                    while (s2 >> 16) s2 = (s2 & 0xffff) + (s2 >> 16);
                    const c2 = (~s2) & 0xffff;
                    f[l4 + 16] = c2 >> 8; f[l4 + 17] = c2 & 0xff;
                    replies.push(f);
                    return replies;
                }
                return replies;
            }
            if (sport === 7 && cliFwIp !== null) {
                // Server-side frames (firmware listens on 7): drive the
                // client role — SYN-ACK -> ACK + data, echo-ACK ignored,
                // FIN -> FIN-ACK. sport 7 never belongs to echo clients.
                const seq = (frame[ipStart + 4] << 24) | (frame[ipStart + 5] << 16) | (frame[ipStart + 6] << 8) | frame[ipStart + 7];
                const fl = frame[ipStart + 13];
                const th = ((frame[ipStart + 12] >> 4) & 0x0f) * 4;
                const ipTot = (frame[16] << 8) | frame[17];
                const dlen = Math.max(0, Math.min(frame.length - ipStart - th, ipTot - (ipStart - 14) - th));
                if ((fl & 0x12) === 0x12) { // SYN-ACK: ACK it, then send data
                    cliSrvIss = seq;
                    const ack = seq + 1;
                    const payload = new TextEncoder().encode('SRV');
                    log('TCP client: SYN-ACK -> ACK + data');
                    replies.push(tcpSeg(cliSport, 7, 0x10, cliSeq + 1, ack, null, srcIp));
                    replies.push(tcpSeg(cliSport, 7, 0x18, cliSeq + 1, ack, payload, srcIp));
                } else if (fl & 0x01) { // FIN: FIN-ACK
                    const ack = seq + dlen + 1;
                    log('TCP client: FIN -> FIN-ACK');
                    replies.push(tcpSeg(cliSport, 7, 0x11, cliSeq + 1 + 3, ack, null, srcIp));
                } else if (dlen > 0 && (fl & 0x10)) { // echo data: ACK it
                    log('TCP client: echo data ACK');
                    replies.push(tcpSeg(cliSport, 7, 0x10, cliSeq + 1 + 3, seq + dlen, null, srcIp));
                }
                return replies;
            }
            if (dport === 7 || (sport === echoSport && echoSport !== 0)) {
                // TCP echo service (port 7): SYN -> SYN-ACK, data -> echo.
                const seq = (frame[ipStart + 4] << 24) | (frame[ipStart + 5] << 16) | (frame[ipStart + 6] << 8) | frame[ipStart + 7];
                const fl = frame[ipStart + 13];
                const th = ((frame[ipStart + 12] >> 4) & 0x0f) * 4;
                // Payload length from the IP total length (NOT the frame
                // length — short frames are zero-padded to 60B on the wire
                // and padding is not payload). Untagged TCP here, so the
                // IP header starts at 14 and L4 at ipStart.
                const ipTot = (frame[16] << 8) | frame[17];
                const dlen = Math.max(0, Math.min(frame.length - ipStart - th, ipTot - (ipStart - 14) - th));
            if (fl === 0x02) { // SYN
                echoSport = sport;
                log('TCP echo SYN -> SYN-ACK');
                replies.push(tcpSeg(7, sport, 0x12, echoSeq, seq + 1, null, srcIp));
                echoSeq = (echoSeq + 1) >>> 0; // SYN consumes one sequence number
            } else if (dlen > 0 && (fl & 0x10)) { // data -> ACK + echo
                const payload = frame.subarray(ipStart + th, ipStart + th + dlen);
                log('TCP echo (' + dlen + 'B)');
                replies.push(tcpSeg(7, sport, 0x10, echoSeq, seq + dlen, null, srcIp));
                replies.push(tcpSeg(7, sport, 0x18, echoSeq, seq + dlen, payload, srcIp));
                echoSeq = (echoSeq + dlen) >>> 0;
            } else if (fl & 0x01) { // FIN -> FIN-ACK
                log('TCP echo FIN -> FIN-ACK');
                replies.push(tcpSeg(7, sport, 0x11, echoSeq, seq + dlen + 1, null, srcIp));
                echoSeq = (echoSeq + 1) >>> 0;
            }
            return replies;
            }
            if (sport !== 0 || dport !== HTTP_PORT) {
                // client -> server frames; learn the ephemeral port
                if (dport === HTTP_PORT) tcpSrcPort = sport;
                if (tcpSrcPort === 0) return replies;
            }
            const seq = (frame[ipStart + 4] << 24) | (frame[ipStart + 5] << 16) | (frame[ipStart + 6] << 8) | frame[ipStart + 7];
            const fl = frame[ipStart + 13];
            const th = ((frame[ipStart + 12] >> 4) & 0x0f) * 4;
            // Same padding rule as the echo path (IP total length, not
            // frame length); HTTP GETs are large but stay correct.
            const ipTot = (frame[16] << 8) | frame[17];
            const dlen = Math.max(0, Math.min(frame.length - ipStart - th, ipTot - (ipStart - 14) - th));

            if (fl === 0x02) { // SYN
                clientSeq = seq;
                stats.synAcks++;
                log('TCP SYN -> SYN-ACK (seq=' + seq + ')');
                replies.push(tcpFrame(0x12, srvSeq, seq + 1, null));
                srvSeq = (srvSeq + 1) >>> 0; // SYN consumes one sequence number
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
