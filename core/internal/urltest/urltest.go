// Package urltest implements v2rayN-style latency probing for a whole node
// batch: ONE sing-box instance hosts every candidate outbound, and each
// node is probed concurrently through sing-box's own urltest.URLTest over
// plain HTTP (http://www.gstatic.com/generate_204) — no TLS round trip,
// matching how v2rayN measures sing-box nodes. Low default concurrency
// avoids tripping per-IP session limits on single-entry servers.
package urltest

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/common/urltest"
	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/option"
	singjson "github.com/sagernet/sing/common/json"
)

const (
	DefaultURL       = "http://www.gstatic.com/generate_204"
	probeConcurrency = 4 // bursts against one entry server trip session limits
)

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

// Probe hosts all entries in one instance and concurrently URL-tests each.
func Probe(sess *Session) ([]Result, error) {
	if len(sess.Entries) == 0 {
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
	order := make([]string, 0, len(sess.Entries))
	idByTag := map[string]string{}
	for _, e := range sess.Entries {
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
	lanes := probeConcurrency
	if sess.Concurrency > 0 {
		lanes = sess.Concurrency
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
				if !loaded {
					mine = append(mine, Result{ID: id, Error: "outbound missing"})
					continue
				}
				probeCtx, cancel := context.WithTimeout(ctx, timeout)
				delay, err := urltest.URLTest(probeCtx, testURL, outbound)
				cancel()
				if err != nil {
					mine = append(mine, Result{ID: id, Error: err.Error()})
					continue
				}
				ms := int64(delay)
				mine = append(mine, Result{ID: id, DelayMs: &ms})
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
	sorted := make([]Result, 0, len(order))
	for _, e := range sess.Entries {
		if r, ok := byID[e.ID]; ok {
			sorted = append(sorted, r)
		} else {
			sorted = append(sorted, Result{ID: e.ID, Error: "untested"})
		}
	}
	return sorted, nil
}
