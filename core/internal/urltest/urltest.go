// Package urltest implements v2rayN-style latency probing for a whole node
// batch: ONE sing-box instance hosts every candidate outbound, and each
// node is probed concurrently through the experimental Clash API delay
// endpoint (which runs the core's own urltest.URLTest against the default
// http://www.gstatic.com/generate_204). No per-node cold start, no local
// socks hop, no extra TLS round trip.
package urltest

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"time"

	box "github.com/sagernet/sing-box"
	_ "github.com/sagernet/sing-box/experimental" // registers services
	_ "github.com/sagernet/sing-box/experimental/cachefile"
	_ "github.com/sagernet/sing-box/experimental/clashapi"
	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/option"
	singjson "github.com/sagernet/sing/common/json"
)

const (
	DefaultURL       = "http://www.gstatic.com/generate_204"
	probeConcurrency = 4 // high concurrency trips per-IP session limits
)

// Entry mirrors build.Entry: a stable id plus its outbound options JSON.
type Entry struct {
	ID  string          `json:"id"`
	Out json.RawMessage `json:"out"`
}

// Session is the request payload read on stdin.
type Session struct {
	Entries  []Entry `json:"entries"`
	TimeoutS int     `json:"timeout_s,omitempty"` // per-node seconds, default 5
	// Concurrency caps simultaneous delay probes (default 32).
	Concurrency int `json:"concurrency,omitempty"`
}

// Result is one node's latency outcome.
type Result struct {
	ID      string `json:"id"`
	DelayMs *int64 `json:"delay_ms,omitempty"`
	Error   string `json:"error,omitempty"`
}

func tagFor(id string) string { return "out-" + id }

// Probe hosts all entries in one instance and concurrently delays each.
func Probe(sess *Session) ([]Result, error) {
	if len(sess.Entries) == 0 {
		return nil, fmt.Errorf("no entries to test")
	}
	timeout := time.Duration(sess.TimeoutS) * time.Second
	if sess.TimeoutS <= 0 {
		timeout = 5 * time.Second
	}

	listen, err := freeTCPPort()
	if err != nil {
		return nil, err
	}
	tokenBytes := make([]byte, 16)
	_, _ = rand.Read(tokenBytes)
	secret := hex.EncodeToString(tokenBytes)
	cachePath := filepath.Join(os.TempDir(), fmt.Sprintf("nekos-urltest-%d.db", time.Now().UnixNano()))
	defer os.Remove(cachePath)

	outbounds := []any{
		map[string]any{"type": "direct", "tag": "direct"},
		map[string]any{"type": "block", "tag": "block"},
	}
	tags := make([]string, 0, len(sess.Entries))
	idByTag := map[string]string{}
	for _, e := range sess.Entries {
		var out map[string]any
		if err := json.Unmarshal(e.Out, &out); err != nil {
			return nil, fmt.Errorf("entry %q: %w", e.ID, err)
		}
		tag := tagFor(e.ID)
		out["tag"] = tag
		outbounds = append(outbounds, out)
		tags = append(tags, tag)
		idByTag[tag] = e.ID
	}

	controller := net.JoinHostPort("127.0.0.1", strconv.Itoa(listen))
	cfg := map[string]any{
		"outbounds": outbounds,
		"route":     map[string]any{"final": "direct"},
		"experimental": map[string]any{
			"cache_file": map[string]any{
				"enabled": true,
				"path":    cachePath,
			},
			"clash_api": map[string]any{
				"external_controller": controller,
				"secret":              secret,
			},
		},
	}
	raw, err := json.Marshal(cfg)
	if err != nil {
		return nil, err
	}
	ctx := include.Context(context.Background())
	opts, err := singjson.UnmarshalExtendedContext[option.Options](ctx, raw)
	if err != nil {
		return nil, fmt.Errorf("invalid probe config: %w", err)
	}

	instance, err := box.New(box.Options{Context: ctx, Options: opts})
	if err != nil {
		return nil, fmt.Errorf("create probe core: %w", err)
	}
	if err := instance.Start(); err != nil {
		return nil, fmt.Errorf("start probe core: %w", err)
	}
	defer instance.Close()

	baseURL := fmt.Sprintf("http://%s", controller)
	client := &http.Client{Timeout: timeout + 5*time.Second}

	results := make([]Result, len(sess.Entries))
	indexByID := make(map[string]int, len(sess.Entries))
	for i, e := range sess.Entries {
		indexByID[e.ID] = i
		results[i] = Result{ID: e.ID}
	}

	// Concurrent delay probes (bounded lanes). High concurrency against a
	// single-entry server can trip per-IP session limits, so allow tuning.
	work := make(chan string)
	lanes := probeConcurrency
	if sess.Concurrency > 0 && sess.Concurrency < lanes {
		lanes = sess.Concurrency
	}
	if lanes > len(sess.Entries) {
		lanes = len(sess.Entries)
	}
	done := make(chan struct{})
	for range lanes {
		go func() {
			defer func() { done <- struct{}{} }()
			for tag := range work {
				req, err := http.NewRequest(http.MethodGet,
					fmt.Sprintf("%s/proxies/%s/delay?timeout=%d", baseURL, tag, timeout.Milliseconds()), nil)
				if err != nil {
					continue
				}
				req.Header.Set("Authorization", "Bearer "+secret)
				resp, err := client.Do(req)
				if err != nil {
					idx := indexByID[idByTag[tag]]
					results[idx] = Result{ID: idByTag[tag], Error: err.Error()}
					continue
				}
				if resp.StatusCode == http.StatusOK {
					var body struct {
						Delay int64 `json:"delay"`
					}
					_ = json.NewDecoder(resp.Body).Decode(&body)
					resp.Body.Close()
					idx := indexByID[idByTag[tag]]
					d := body.Delay
					results[idx] = Result{ID: idByTag[tag], DelayMs: &d}
					continue
				}
				resp.Body.Close()
				idx := indexByID[idByTag[tag]]
				results[idx] = Result{ID: idByTag[tag], Error: "unavailable"}
			}
		}()
	}
	for _, tag := range tags {
		work <- tag
	}
	close(work)
	for range lanes {
		<-done
	}
	return results, nil
}

func freeTCPPort() (int, error) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return 0, err
	}
	defer l.Close()
	return l.Addr().(*net.TCPAddr).Port, nil
}
