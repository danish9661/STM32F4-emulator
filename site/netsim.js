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
    let echoSport = 0;  // TCP-echo (port 7) client port, learned from SYN
    let echoSeq = 0x20000000; // TCP-echo server ISN
    // TCP-server-test client role (lwip_demo listens on 7): fixed sport,
    // connects when the firmware sends a UDP trigger to port 5004.
    let cliSport = 5005, cliSeq = 0x30000000, cliSrvIss = 0, cliFwIp = null;
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
            }
            return replies;
        }

        if (proto === 1) { // ICMP: echo request -> echo reply (any dst IP)
            const type = frame[ipStart];
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
