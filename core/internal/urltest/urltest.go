// Package urltest implements latency probing for a whole node batch: ONE
// sing-box instance hosts every candidate outbound, and each node is
// probed concurrently through sing-box's own urltest.URLTest over plain
// HTTP (http://www.gstatic.com/generate_204). The delay is timed AFTER
// the transport handshake (for handshake-required protocols), which is
// exactly the number v2rayN's sing-box backend reports — keeping the two
// tools comparable. Low default concurrency avoids tripping per-IP session
// limits on single-entry servers.
package urltest

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/common/urltest"
	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/option"
	singjson "github.com/sagernet/sing/common/json"
)

const DefaultURL = "http://www.gstatic.com/generate_204"

// Entry mirrors build.Entry: a stable id plus its outbound options JSON.
type Entry struct {
	ID  string          `json:"id"`
	Out json.RawMessage `json:"out"`
}

// Session is the request payload read on stdin.
type Session struct {
	Entries  []Entry `json:"entries"`
	URL      string  `json:"url,omitempty"`
	TimeoutS int     `json:"timeout_s,omitempty"` // per-node seconds, default 5
	// Concurrency caps simultaneous probes (default 4).
	Concurrency int `json:"concurrency,omitempty"`
}

// Result is one node's latency outcome.
type Result struct {
	ID      string `json:"id"`
	DelayMs *int64 `json:"delay_ms,omitempty"`
	Error   string `json:"error,omitempty"`
}

func tagFor(id string) string { return "out-" + id }

// Sink receives each node's result as soon as it is measured (including
// retry-round replacements) — used to stream batch progress to callers
// while Probe is still running.
type Sink func(Result)

// isTransient reports whether an error looks like a burst-timeout on the
// exit server rather than a deterministic node problem (HTTP status, dead
// DNS). Only these are retried: a node that times out once under the batch
// burst is normally fine on a solo test.
func isTransient(err string) bool {
	for _, token := range []string{"timeout", "deadline", "closed", "reset"} {
		if strings.Contains(err, token) {
			return true
		}
	}
	return false
}

// Probe hosts all entries in one instance and concurrently URL-tests each.
func Probe(sess *Session) ([]Result, error) { return ProbeWithSink(sess, nil) }

// ProbeWithSink is Probe with per-node progress reporting: every measured
// node is handed to sink the moment it finishes. Entries whose first
// attempt fails with a transient error (burst timeout, closed connection)
// are retried once in a second, gentler round — solo re-tests of such
// nodes succeed, and reporting them as failed would mark valid nodes dead.
func ProbeWithSink(sess *Session, sink Sink) ([]Result, error) {
	if len(sess.Entries) == 0 {
		return nil, fmt.Errorf("no entries to test")
	}
	round1, err := probeOnce(sess, sess.Entries, sink)
	if err != nil {
		return nil, err
	}
	byID := make(map[string]Result, len(round1))
	for _, r := range round1 {
		byID[r.ID] = r
	}
	var retry []Entry
	for _, e := range sess.Entries {
		if r, ok := byID[e.ID]; ok && r.Error != "" && isTransient(r.Error) {
			retry = append(retry, e)
		}
	}
	if len(retry) > 0 {
		// Gentler + slower retry round: the failure was burst-related, so
		// give the second attempt more headroom (up to 20s) at lower
		// concurrency instead of clipping it at the same per-node cap.
		retryTimeout := 10
		if sess.TimeoutS > 0 {
			retryTimeout = sess.TimeoutS * 2
			if retryTimeout > 20 {
				retryTimeout = 20
			}
		}
		sub := &Session{
			URL:         sess.URL,
			TimeoutS:    retryTimeout,
			Concurrency: 2,
		}
		if sess.Concurrency > 0 && sess.Concurrency < 2 {
			sub.Concurrency = sess.Concurrency
		}
		sub.Entries = retry
		round2, err := probeOnce(sub, retry, sink)
		if err == nil {
			for _, r := range round2 {
				byID[r.ID] = r
			}
		}
	}
	sorted := make([]Result, 0, len(sess.Entries))
	for _, e := range sess.Entries {
		if r, ok := byID[e.ID]; ok {
			sorted = append(sorted, r)
		} else {
			sorted = append(sorted, Result{ID: e.ID, Error: "untested"})
		}
	}
	return sorted, nil
}

// probeOnce runs one probing round over entries with a fresh instance and
// returns results aligned with the input order. Each completed node is
// reported through sink (nil allowed).
func probeOnce(sess *Session, entries []Entry, sink Sink) ([]Result, error) {
	if len(entries) == 0 {
		return nil, fmt.Errorf("no entries to test")
	}
	testURL := sess.URL
	if testURL == "" {
		testURL = DefaultURL
	}
	timeout := time.Duration(sess.TimeoutS) * time.Second
	if sess.TimeoutS <= 0 {
		timeout = 5 * time.Second
	}

	outbounds := []any{
		map[string]any{"type": "direct", "tag": "direct"},
		map[string]any{"type": "block", "tag": "block"},
	}
	order := make([]string, 0, len(entries))
	idByTag := map[string]string{}
	for _, e := range entries {
		var out map[string]any
		if err := json.Unmarshal(e.Out, &out); err != nil {
			return nil, fmt.Errorf("entry %q: %w", e.ID, err)
		}
		tag := tagFor(e.ID)
		out["tag"] = tag
		outbounds = append(outbounds, out)
		order = append(order, tag)
		idByTag[tag] = e.ID
	}

	cfg := map[string]any{
		"outbounds": outbounds,
		"route":     map[string]any{"final": "direct"},
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
	if opts.Log == nil {
		opts.Log = &option.LogOptions{}
	}
	// Logs land in the daemon's stderr (the app log panel). Without a log
	// block sing-box defaults to TRACE, which floods the panel on every
	// batch test; the probe's diagnostics already come back via RPC rows,
	// so cap this instance at error.
	opts.Log.Level = "error"
	opts.Log.DisableColor = true

	instance, err := box.New(box.Options{Context: ctx, Options: opts})
	if err != nil {
		return nil, fmt.Errorf("create probe core: %w", err)
	}
	if err := instance.Start(); err != nil {
		return nil, fmt.Errorf("start probe core: %w", err)
	}
	defer instance.Close()

	results := make([]Result, 0, len(order))
	work := make(chan string)
	lanes := sess.Concurrency
	if lanes == 0 {
		// Default 4 lanes; large cross-server batches (many distinct
		// hosts) scale up safely — 24+ entries are never one hot server.
		if len(order) > 24 {
			lanes = 10
		} else {
			lanes = 4
		}
	}
	if lanes > len(order) {
		lanes = len(order)
	}
	done := make(chan []Result, lanes)
	for range lanes {
		go func() {
			var mine []Result
			defer func() { done <- mine }()
			for tag := range work {
				id := idByTag[tag]
				outbound, loaded := instance.Outbound().Outbound(tag)
				var res Result
				if !loaded {
					res = Result{ID: id, Error: "outbound missing"}
				} else {
					probeCtx, cancel := context.WithTimeout(ctx, timeout)
					delay, err := urltest.URLTest(probeCtx, testURL, outbound)
					cancel()
					if err != nil {
						res = Result{ID: id, Error: err.Error()}
					} else {
						ms := int64(delay)
						res = Result{ID: id, DelayMs: &ms}
					}
				}
				if sink != nil {
					sink(res)
				}
				mine = append(mine, res)
			}
		}()
	}
	for _, tag := range order {
		work <- tag
	}
	close(work)
	for range lanes {
		results = append(results, <-done...)
	}

	byID := make(map[string]Result, len(results))
	for _, r := range results {
		byID[r.ID] = r
	}
	sorted := make([]Result, 0, len(entries))
	for _, e := range entries {
		if r, ok := byID[e.ID]; ok {
			sorted = append(sorted, r)
		} else {
			sorted = append(sorted, Result{ID: e.ID, Error: "untested"})
		}
	}
	return sorted, nil
}
