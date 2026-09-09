// nekos-core is the sing-box control process for the nekos Tauri client.
// It embeds upstream sing-box as a library and exposes a small CLI today;
// the JSON-RPC surface (architecture.md §6) lands on top of the same
// build/run packages.
package main

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/signal"
	"runtime"
	"runtime/debug"
	"strconv"
	"syscall"
	"time"

	qrcode "github.com/skip2/go-qrcode"

	"nekos/core/internal/build"
	"nekos/core/internal/link"
	"nekos/core/internal/rpc"
	"nekos/core/internal/run"
	"nekos/core/internal/urltest"
)

func main() {
	if len(os.Args) < 2 {
		usage()
		os.Exit(2)
	}
	var err error
	switch os.Args[1] {
	case "parse":
		err = cmdParse()
	case "config":
		err = cmdConfig()
	case "run":
		err = cmdRun()
	case "serve":
		err = cmdServe(os.Args[2:])
	case "test":
		err = cmdTest(os.Args[2:])
	case "urltest":
		err = cmdURLTest()
	case "encode":
		err = cmdEncode()
	case "qr":
		err = cmdQR()
	case "version":
		err = cmdVersion()
	case "help", "-h", "--help":
		usage()
	default:
		fmt.Fprintf(os.Stderr, "nekos-core: unknown command %q\n", os.Args[1])
		usage()
		os.Exit(2)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "nekos-core:", err)
		os.Exit(1)
	}
}

func usage() {
	fmt.Print(`nekos-core — sing-box control process

usage:
  nekos-core parse               read links/subscription text on stdin,
                                 print import result (nodes + errors) as JSON
  nekos-core config              read a session JSON on stdin, print the
                                 assembled (validated) sing-box config JSON
  nekos-core run                 read a session JSON on stdin and run until
                                 SIGINT/SIGTERM
  nekos-core serve --rpc LISTEN --token TOKEN
                                 long-lived JSON-RPC control process
                                 (architecture.md §6); prints its bound
                                 address as {"rpc": "127.0.0.1:PORT"} on
                                 stdout once the listener is up
  nekos-core test [-target HOST:PORT] [-timeout SECONDS]
                                 read a session JSON on stdin, measure TCP
                                 latency through the selected node, print
                                 {"delay_ms": N}
  nekos-core urltest             read {"entries":[...], "url"?, "timeout_s"?}
                                 on stdin; batch-probe all entries in one
                                 sing-box instance (urltest group, HTTP
                                 generate_204), print per-node delays
  nekos-core encode               read {"remark": "...", "out": {...}} on
                                 stdin, print the share link for the node
                                 (anytls/trojan/vless/ss)
  nekos-core qr                   same input as encode (plus optional
                                 "size"), print base64 PNG QR code of the
                                 share link
  nekos-core version             print version and embedded sing-box module
`)
}

// ---- stdin helpers ----

func readStdin() ([]byte, error) {
	data, err := io.ReadAll(os.Stdin)
	if err != nil {
		return nil, fmt.Errorf("read stdin: %w", err)
	}
	return data, nil
}

func decodeSession() (*build.Session, error) {
	data, err := readStdin()
	if err != nil {
		return nil, err
	}
	var sess build.Session
	if err := json.Unmarshal(data, &sess); err != nil {
		return nil, fmt.Errorf("decode session: %w", err)
	}
	if sess.Mode == "" {
		sess.Mode = "global"
	}
	if sess.LogLevel == "" {
		sess.LogLevel = "warn"
	}
	return &sess, nil
}

// ---- commands ----

func cmdParse() error {
	data, err := readStdin()
	if err != nil {
		return err
	}
	result := link.Parse(string(data))
	out, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}
	fmt.Println(string(out))
	return nil
}

func cmdConfig() error {
	sess, err := decodeSession()
	if err != nil {
		return err
	}
	raw, err := build.AssembleJSON(sess)
	if err != nil {
		return err
	}
	var pretty map[string]any
	_ = json.Unmarshal(raw, &pretty)
	out, _ := json.MarshalIndent(pretty, "", "  ")
	fmt.Println(string(out))
	return nil
}

func cmdRun() error {
	sess, err := decodeSession()
	if err != nil {
		return err
	}
	m := run.NewManager()
	if err := m.Start(sess); err != nil {
		return err
	}
	status, _ := json.Marshal(map[string]any{
		"running":    true,
		"started_at": m.StartedAt().Format(time.RFC3339),
	})
	fmt.Println(string(status))

	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	<-sig
	return m.Stop()
}

// cmdServe runs the long-lived JSON-RPC control process (architecture.md
// §6.1). The orchestrator passes a loopback listen address (127.0.0.1:0 for
// an ephemeral port) and a random token; the bound address is printed as a
// single JSON line on stdout so the orchestrator can start talking.
func cmdServe(args []string) error {
	listen := "127.0.0.1:0"
	token := ""
	parentPid := 0
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "--rpc":
			i++
			if i >= len(args) {
				return fmt.Errorf("--rpc needs a value")
			}
			listen = args[i]
		case "--token":
			i++
			if i >= len(args) {
				return fmt.Errorf("--token needs a value")
			}
			token = args[i]
		case "--parent-pid":
			i++
			if i >= len(args) {
				return fmt.Errorf("--parent-pid needs a value")
			}
			n, err := strconv.Atoi(args[i])
			if err != nil || n <= 0 {
				return fmt.Errorf("invalid --parent-pid %q", args[i])
			}
			parentPid = n
		default:
			return fmt.Errorf("unknown flag %q", args[i])
		}
	}
	if token == "" {
		return errors.New("serve requires --token")
	}
	ln, err := net.Listen("tcp", listen)
	if err != nil {
		return fmt.Errorf("listen %s: %w", listen, err)
	}
	m := run.NewManager()
	srv := rpc.NewServer(token, m)
	go func() {
		_ = srv.Serve(ln)
	}()
	out, _ := json.Marshal(map[string]any{
		"running": false,
		"rpc":     ln.Addr().String(),
	})
	fmt.Println(string(out))

	// Die with the controlling GUI: a crashed orchestrator must not leave
	// this daemon (and the sing-box instance inside it, which owns the
	// inbound port and system-proxy route) running as an orphan that a new
	// GUI cannot take over. The orchestrator passes its own pid; once this
	// process has been reparented (direct parent exited) it is orphaned and
	// shuts down. Windows does not reparent on parent death, so the check
	// is skipped there (ppid stays stale).
	if parentPid > 0 && runtime.GOOS != "windows" {
		go watchParent(parentPid)
	}

	sig := make(chan os.Signal, 1)
	signal.Notify(sig, syscall.SIGINT, syscall.SIGTERM)
	<-sig
	_ = srv.Close()
	return m.Stop()
}

// watchParent polls until this process is no longer a direct child of pid
// (i.e. its parent exited and it got reparented), then exits so the daemon
// never outlives the GUI that started it.
func watchParent(parent int) {
	t := time.NewTicker(2 * time.Second)
	defer t.Stop()
	for range t.C {
		if os.Getppid() != parent {
			fmt.Fprintf(os.Stderr, "nekos-core: controlling process (pid %d) exited; shutting down\n", parent)
			os.Exit(0)
		}
	}
}

func cmdTest(args []string) error {
	target := "https://www.google.com/generate_204"
	timeout := 5 * time.Second
	for i := 0; i < len(args); i++ {
		switch args[i] {
		case "-target":
			i++
			if i >= len(args) {
				return fmt.Errorf("-target needs a value")
			}
			target = args[i]
		case "-timeout":
			i++
			if i >= len(args) {
				return fmt.Errorf("-timeout needs a value")
			}
			secs, err := strconv.ParseFloat(args[i], 64)
			if err != nil {
				return fmt.Errorf("invalid -timeout: %w", err)
			}
			timeout = time.Duration(secs * float64(time.Second))
		default:
			return fmt.Errorf("unknown flag %q", args[i])
		}
	}
	sess, err := decodeSession()
	if err != nil {
		return err
	}
	delay, err := run.MeasureNode(sess, target, timeout)
	if err != nil {
		return err
	}
	out, _ := json.Marshal(map[string]any{
		"delay_ms":  delay.Milliseconds(),
		"delay_ns":  delay.Nanoseconds(),
		"target":    target,
		"timeout_s": timeout.Seconds(),
	})
	fmt.Println(string(out))
	return nil
}

func cmdURLTest() error {
	data, err := readStdin()
	if err != nil {
		return err
	}
	var sess urltest.Session
	if err := json.Unmarshal(data, &sess); err != nil {
		return fmt.Errorf("decode urltest session: %w", err)
	}
	results, err := urltest.Probe(&sess)
	if err != nil {
		return err
	}
	out, err := json.MarshalIndent(results, "", "  ")
	if err != nil {
		return err
	}
	fmt.Println(string(out))
	return nil
}

func cmdEncode() error {
	var req struct {
		Remark string          `json:"remark"`
		Out    json.RawMessage `json:"out"`
	}
	if err := json.NewDecoder(os.Stdin).Decode(&req); err != nil {
		return fmt.Errorf("decode encode request: %w", err)
	}
	link, err := link.Encode(req.Remark, req.Out)
	if err != nil {
		return err
	}
	fmt.Println(link)
	return nil
}

func cmdQR() error {
	var req struct {
		Remark string          `json:"remark"`
		Out    json.RawMessage `json:"out"`
		Size   int             `json:"size"`
	}
	if err := json.NewDecoder(os.Stdin).Decode(&req); err != nil {
		return fmt.Errorf("decode qr request: %w", err)
	}
	link, err := link.Encode(req.Remark, req.Out)
	if err != nil {
		return err
	}
	size := req.Size
	if size < 128 || size > 2048 {
		size = 300
	}
	png, err := qrcode.Encode(link, qrcode.Medium, size)
	if err != nil {
		return fmt.Errorf("qr encode: %w", err)
	}
	fmt.Print(base64.StdEncoding.EncodeToString(png))
	return nil
}

func cmdVersion() error {
	bi, ok := debug.ReadBuildInfo()
	if !ok {
		fmt.Println("nekos-core (build info unavailable)")
		return nil
	}
	coreVersion := "unknown"
	for _, dep := range bi.Deps {
		if dep.Path == "github.com/sagernet/sing-box" {
			coreVersion = dep.Version
			break
		}
	}
	fmt.Printf("nekos-core %s (go %s)\n", bi.Main.Version, bi.GoVersion)
	fmt.Printf("sing-box %s\n", coreVersion)
	return nil
}
