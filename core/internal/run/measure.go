package run

import (
	"bufio"
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"io"
	"net"
	"strconv"
	"strings"
	"time"

	"nekos/core/internal/build"
)

// MeasureNode boots an ephemeral socks listener whose traffic is routed to
// the session's selected node, then measures the selected node's latency:
//
//   - "tcp://host:port" / "host:port": SOCKS CONNECT timing (v2rayN tcp
//     ping semantics; for ws/grpc transports CONNECT returns before the
//     server-side channel is verified).
//   - "http://host[:port]/path" / "https://…": CONNECT + raw HTTP GET; the
//     measured time is until the response status line arrives, exercising
//     the full tunnel (including remote TLS/WS handshakes).
func MeasureNode(sess *build.Session, target string, timeout time.Duration) (time.Duration, error) {
	if sess.Mode != "global" || sess.Selected == "" {
		return 0, errors.New("measure requires mode=global and a selected entry")
	}
	kind, host, port, path, err := parseTarget(target)
	if err != nil {
		return 0, err
	}
	freePort, err := freeTCPPort()
	if err != nil {
		return 0, fmt.Errorf("no free port for probe listener: %w", err)
	}

	probe := *sess
	probe.Inbound = build.InboundConfig{Listen: "127.0.0.1", Port: freePort, Type: "socks"}

	m := NewManager()
	if err := m.Start(&probe); err != nil {
		return 0, fmt.Errorf("start probe core: %w", err)
	}
	defer m.Stop()

	addr := net.JoinHostPort("127.0.0.1", strconv.Itoa(int(freePort)))
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	start := time.Now()
	conn, err := socks5Dial(ctx, addr, host, port)
	if err != nil {
		return 0, err
	}
	defer conn.Close()
	if kind != targetTCP {
		if err := httpProbe(ctx, conn, host, port, path, kind == targetHTTPS); err != nil {
			return 0, err
		}
	}
	return time.Since(start), nil
}

type targetKind int

const (
	targetTCP targetKind = iota
	targetHTTP
	targetHTTPS
)

// parseTarget parses tcp://, http:// and https:// test targets plus the
// bare host:port form. Returns kind, host, port and (for http(s)) path.
func parseTarget(target string) (targetKind, string, int, string, error) {
	t := strings.TrimSpace(target)
	kind := targetTCP
	path := "/"
	switch {
	case strings.HasPrefix(t, "https://"):
		kind = targetHTTPS
		t = strings.TrimPrefix(t, "https://")
	case strings.HasPrefix(t, "http://"):
		kind = targetHTTP
		t = strings.TrimPrefix(t, "http://")
	case strings.HasPrefix(t, "tcp://"):
		t = strings.TrimPrefix(t, "tcp://")
	}
	if kind != targetTCP {
		if idx := strings.IndexByte(t, '/'); idx >= 0 {
			t, path = t[:idx], t[idx:]
		}
		if path == "" {
			path = "/"
		}
	}
	host, portStr, err := net.SplitHostPort(t)
	if err != nil {
		if kind == targetTCP || strings.Contains(t, ":") {
			return 0, "", 0, "", fmt.Errorf("invalid test target %q (want tcp://host:port)", target)
		}
		host = t // URL without explicit port
		portStr = "80"
		if kind == targetHTTPS {
			portStr = "443"
		}
	}
	port, err := strconv.Atoi(portStr)
	if err != nil || port <= 0 || port > 65535 {
		return 0, "", 0, "", fmt.Errorf("invalid test target port %q", portStr)
	}
	return kind, host, port, path, nil
}

// httpProbe performs a minimal GET over an established tunnel (layering
// TLS first for https targets) and verifies a <400 HTTP status. The
// response status line is the proof that the full node path answers.
func httpProbe(ctx context.Context, conn net.Conn, host string, port int, path string, secure bool) error {
	writeConn := conn
	if secure {
		serverName := strings.TrimSuffix(strings.TrimPrefix(host, "["), "]")
		tlsConn := tls.Client(conn, &tls.Config{
			ServerName: serverName,
			MinVersion: tls.VersionTLS12,
		})
		if deadline, ok := ctx.Deadline(); ok {
			_ = tlsConn.SetDeadline(deadline)
		}
		if err := tlsConn.HandshakeContext(ctx); err != nil {
			return fmt.Errorf("probe tls handshake: %w", err)
		}
		defer tlsConn.Close()
		writeConn = tlsConn
	}
	hostHeader := host
	if (secure && port != 443) || (!secure && port != 80) {
		hostHeader = net.JoinHostPort(host, strconv.Itoa(port))
	}
	req := "GET " + path + " HTTP/1.1\r\n" +
		"Host: " + hostHeader + "\r\n" +
		"User-Agent: nekos-core/0.1\r\n" +
		"Accept: */*\r\n" +
		"Connection: close\r\n\r\n"
	if _, err := writeConn.Write([]byte(req)); err != nil {
		return fmt.Errorf("probe request write: %w", err)
	}
	reader := bufio.NewReader(writeConn)
	statusLine, err := reader.ReadString('\n')
	if err != nil {
		return fmt.Errorf("probe response read: %w", err)
	}
	statusLine = strings.TrimSpace(statusLine)
	parts := strings.SplitN(statusLine, " ", 3)
	if len(parts) < 2 || !strings.HasPrefix(parts[0], "HTTP/") {
		return fmt.Errorf("probe: malformed status line %q", statusLine)
	}
	code, err := strconv.Atoi(parts[1])
	if err != nil {
		return fmt.Errorf("probe: bad status code %q", parts[1])
	}
	if code >= 400 {
		return fmt.Errorf("probe: upstream returned HTTP %d", code)
	}
	return nil
}

func freeTCPPort() (uint16, error) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return 0, err
	}
	defer l.Close()
	return uint16(l.Addr().(*net.TCPAddr).Port), nil
}

// socks5Dial performs a no-auth SOCKS5 CONNECT through the probe listener
// and returns the established tunnel. The measured path is: loopback ->
// sing-box socks inbound -> selected node outbound -> target.
func socks5Dial(ctx context.Context, proxyAddr, host string, port int) (net.Conn, error) {
	d := net.Dialer{}
	conn, err := d.DialContext(ctx, "tcp", proxyAddr)
	if err != nil {
		return nil, fmt.Errorf("probe dial: %w", err)
	}
	fail := func(e error) (net.Conn, error) {
		conn.Close()
		return nil, e
	}
	if deadline, ok := ctx.Deadline(); ok {
		_ = conn.SetDeadline(deadline)
	}

	if _, err := conn.Write([]byte{0x05, 0x01, 0x00}); err != nil {
		return fail(fmt.Errorf("socks greeting: %w", err))
	}
	var resp [2]byte
	if _, err := io.ReadFull(conn, resp[:]); err != nil {
		return fail(fmt.Errorf("socks greeting reply: %w", err))
	}
	if resp[0] != 0x05 || resp[1] != 0x00 {
		return fail(fmt.Errorf("socks server rejected auth method (%d)", resp[1]))
	}

	req := []byte{0x05, 0x01, 0x00, 0x03, byte(len(host))}
	req = append(req, host...)
	req = append(req, byte(port>>8), byte(port&0xff))
	if _, err := conn.Write(req); err != nil {
		return fail(fmt.Errorf("socks connect: %w", err))
	}

	var header [4]byte
	if _, err := io.ReadFull(conn, header[:]); err != nil {
		return fail(fmt.Errorf("socks connect reply: %w", err))
	}
	if header[0] != 0x05 {
		return fail(fmt.Errorf("socks bad version in reply"))
	}
	if header[1] != 0x00 {
		return fail(socksError(header[1]))
	}
	// consume BND.ADDR
	switch header[3] {
	case 0x01:
		_, _ = io.CopyN(io.Discard, conn, 4+2)
	case 0x04:
		_, _ = io.CopyN(io.Discard, conn, 16+2)
	case 0x03:
		var l [1]byte
		if _, err := io.ReadFull(conn, l[:]); err != nil {
			return fail(err)
		}
		_, _ = io.CopyN(io.Discard, conn, int64(l[0])+2)
	default:
		return fail(fmt.Errorf("socks bad address type %d", header[3]))
	}
	return conn, nil
}

func socksError(code byte) error {
	switch code {
	case 0x01:
		return errors.New("general socks failure")
	case 0x02:
		return errors.New("connection not allowed by ruleset")
	case 0x03:
		return errors.New("network unreachable")
	case 0x04:
		return errors.New("host unreachable")
	case 0x05:
		return errors.New("connection refused")
	case 0x06:
		return errors.New("ttl expired")
	case 0x07:
		return errors.New("command not supported")
	case 0x08:
		return errors.New("address type not supported")
	default:
		return fmt.Errorf("socks failure code %d", code)
	}
}
