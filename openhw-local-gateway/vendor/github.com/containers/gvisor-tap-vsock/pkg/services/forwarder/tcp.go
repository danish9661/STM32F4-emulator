package forwarder

import (
	"context"
	"fmt"
	"net"
	"sync"

	"github.com/containers/gvisor-tap-vsock/pkg/tcpproxy"
	log "github.com/sirupsen/logrus"
	"gvisor.dev/gvisor/pkg/tcpip"
	"gvisor.dev/gvisor/pkg/tcpip/adapters/gonet"
	"gvisor.dev/gvisor/pkg/tcpip/stack"
	"gvisor.dev/gvisor/pkg/tcpip/transport/tcp"
	"gvisor.dev/gvisor/pkg/waiter"
)

func dbg(format string, args ...interface{}) {
	// Debug disabled
}

const linkLocalSubnet = "169.254.0.0/16"

func TCP(s *stack.Stack, nat map[tcpip.Address]tcpip.Address, natLock *sync.Mutex) *tcp.Forwarder {
	return tcp.NewForwarder(s, 0, 10, func(r *tcp.ForwarderRequest) {
		localAddress := r.ID().LocalAddress
		dbg("Forwarder request: local=%s, port=%d", localAddress, r.ID().LocalPort)

		if linkLocal().Contains(localAddress) {
			dbg("link-local, sending RST")
			r.Complete(true)
			return
		}

		natLock.Lock()
		if replaced, ok := nat[localAddress]; ok {
			dbg("NAT: %s -> %s", localAddress, replaced)
			localAddress = replaced
		} else {
			dbg("NAT: %s not in table (keys=%v)", localAddress, nat)
		}
		natLock.Unlock()
		target := fmt.Sprintf("%s:%d", localAddress, r.ID().LocalPort)
		dbg("Dialing %s ...", target)
		outbound, err := net.Dial("tcp", target)
		if err != nil {
			dbg("net.Dial FAILED: %v", err)
			log.Tracef("net.Dial() = %v", err)
			r.Complete(true)
			return
		}
		dbg("net.Dial OK")

		var wq waiter.Queue
		ep, tcpErr := r.CreateEndpoint(&wq)
		r.Complete(false)
		if tcpErr != nil {
			dbg("CreateEndpoint failed: %v", tcpErr)
			if _, ok := tcpErr.(*tcpip.ErrConnectionRefused); ok {
				// transient error
				log.Debugf("r.CreateEndpoint() = %v", tcpErr)
			} else {
				log.Errorf("r.CreateEndpoint() = %v", tcpErr)
			}
			return
		}
		dbg("CreateEndpoint OK, proxying")

		remote := tcpproxy.DialProxy{
			DialContext: func(_ context.Context, _, _ string) (net.Conn, error) {
				return outbound, nil
			},
		}
		remote.HandleConn(gonet.NewTCPConn(&wq, ep))
	})
}

func linkLocal() *tcpip.Subnet {
	_, parsedSubnet, _ := net.ParseCIDR(linkLocalSubnet) // CoreOS VM tries to connect to Amazon EC2 metadata service
	subnet, _ := tcpip.NewSubnet(tcpip.AddrFromSlice(parsedSubnet.IP), tcpip.MaskFromBytes(parsedSubnet.Mask))
	return &subnet
}
