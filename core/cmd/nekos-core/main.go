// nekos-core is the sing-box control process for the nekos Tauri client.
// It embeds upstream sing-box as a library and exposes a small CLI today;
// the JSON-RPC surface (architecture.md §6) lands on top of the same
// build/run packages.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/signal"
	"runtime/debug"
	"strconv"
	"syscall"
	"time"

	"nekos/core/internal/build"
	"nekos/core/internal/link"
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
	case "test":
		err = cmdTest(os.Args[2:])
	case "urltest":
		err = cmdURLTest()
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
  nekos-core test [-target HOST:PORT] [-timeout SECONDS]
                                 read a session JSON on stdin, measure TCP
                                 latency through the selected node, print
                                 {"delay_ms": N}
  nekos-core urltest             read {"entries":[...], "url"?, "timeout_s"?}
                                 on stdin; batch-probe all entries in one
                                 sing-box instance (urltest group, HTTP
                                 generate_204), print per-node delays
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
