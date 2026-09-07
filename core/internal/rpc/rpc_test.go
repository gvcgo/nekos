package rpc

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"

	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"

	"nekos/core/internal/link"
	"nekos/core/internal/model"
	"nekos/core/internal/run"
)

const testToken = "test-token"

func startTestServer(t *testing.T) string {
	t.Helper()
	mgr := run.NewManager()
	srv := NewServer(testToken, mgr)
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() {
		_ = srv.Close()
		_ = mgr.Stop()
	})
	return "http://" + ln.Addr().String()
}

type rpcResp struct {
	Result json.RawMessage `json:"result"`
	Error  *struct {
		Code    int    `json:"code"`
		Message string `json:"message"`
	} `json:"error"`
}

func call(t *testing.T, base, token, method string, params any) (int, rpcResp) {
	t.Helper()
	body, err := json.Marshal(map[string]any{
		"jsonrpc": "2.0",
		"id":      1,
		"method":  method,
		"params":  params,
	})
	if err != nil {
		t.Fatal(err)
	}
	req, err := http.NewRequest(http.MethodPost, base+"/rpc", bytes.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		t.Fatal(err)
	}
	var out rpcResp
	if err := json.Unmarshal(raw, &out); err != nil {
		t.Fatalf("bad rpc response %q: %v", raw, err)
	}
	return resp.StatusCode, out
}

func mustOK(t *testing.T, base, method string, params any) rpcResp {
	t.Helper()
	status, out := call(t, base, testToken, method, params)
	if status != http.StatusOK {
		t.Fatalf("%s: HTTP %d", method, status)
	}
	if out.Error != nil {
		t.Fatalf("%s: rpc error %d: %s", method, out.Error.Code, out.Error.Message)
	}
	return out
}

// parseNodes parses the sample anytls links used across the suite.
func parseNodes(t *testing.T, links ...string) []*model.Node {
	t.Helper()
	res := link.Parse(strings.Join(links, "\n"))
	if len(res.Nodes) != len(links) || len(res.Errors) > 0 {
		t.Fatalf("parse: got %d nodes, %d errors (want %d)", len(res.Nodes), len(res.Errors), len(links))
	}
	return res.Nodes
}

// startSession builds a global-mode session for one node id.
func startSession(t *testing.T, nodes []*model.Node, selected string) map[string]any {
	t.Helper()
	var entries []any
	for _, n := range nodes {
		var out any
		if err := json.Unmarshal(n.Out, &out); err != nil {
			t.Fatal(err)
		}
		entries = append(entries, map[string]any{"id": n.ID, "out": out})
	}
	return map[string]any{
		"mode":     "global",
		"entries":  entries,
		"selected": selected,
	}
}

func statusOf(t *testing.T, base string) map[string]any {
	t.Helper()
	out := mustOK(t, base, "core.status", map[string]any{})
	var st map[string]any
	if err := json.Unmarshal(out.Result, &st); err != nil {
		t.Fatalf("status decode: %v", err)
	}
	return st
}

func TestAuthRequired(t *testing.T) {
	base := startTestServer(t)
	params := map[string]any{"text": "anytls://e0c664d9-415f-30b5-aeaf-547be3870274@127.0.0.1:26019/#x"}
	for name, token := range map[string]string{"missing": "", "wrong": "nope"} {
		status, _ := call(t, base, token, "parse.text", params)
		if status != http.StatusUnauthorized {
			t.Fatalf("%s token: got HTTP %d, want 401", name, status)
		}
	}
	mustOK(t, base, "parse.text", params)
}

func TestUnknownMethod(t *testing.T) {
	base := startTestServer(t)
	_, out := call(t, base, testToken, "no.such", map[string]any{})
	if out.Error == nil || out.Error.Code != codeMethodNotFound {
		t.Fatalf("want method-not-found error, got %+v", out.Error)
	}
}

func TestParseAndEncode(t *testing.T) {
	base := startTestServer(t)
	out := mustOK(t, base, "parse.text", map[string]any{
		"text": "anytls://e0c664d9-415f-30b5-aeaf-547be3870274@ew.ali66mysql.com:26019/?sni=www.apple.com&insecure=1#解析节点",
	})
	var res struct {
		Nodes  []*model.Node       `json:"nodes"`
		Errors []model.ImportError `json:"errors"`
	}
	if err := json.Unmarshal(out.Result, &res); err != nil {
		t.Fatal(err)
	}
	if len(res.Nodes) != 1 || len(res.Errors) != 0 {
		t.Fatalf("want 1 node, got %d nodes / %d errors", len(res.Nodes), len(res.Errors))
	}
	node := res.Nodes[0]
	if node.Remark != "解析节点" {
		t.Fatalf("remark = %q", node.Remark)
	}
	// round trip: encode the parsed node back to its share link
	enc := mustOK(t, base, "encode", map[string]any{"remark": node.Remark, "out": node.Out})
	var got string
	if err := json.Unmarshal(enc.Result, &got); err != nil {
		t.Fatalf("encode result not a string: %v", err)
	}
	if !strings.HasPrefix(got, "anytls://") {
		t.Fatalf("encode result %q", got)
	}
}

func TestCoreLifecycleAndReplace(t *testing.T) {
	base := startTestServer(t)
	nodes := parseNodes(t,
		"anytls://e0c664d9-415f-30b5-aeaf-547be3870274@ew.ali66mysql.com:26019/?sni=www.apple.com&insecure=1#a",
		"anytls://e0c664d9-415f-30b5-aeaf-547be3870274@10.0.0.2:26019/?sni=www.example.com&insecure=1#b",
	)

	// start
	out := mustOK(t, base, "core.start", map[string]any{"session": startSession(t, nodes, nodes[0].ID)})
	var st struct {
		Running   bool   `json:"running"`
		StartedAt string `json:"started_at"`
		Core      string `json:"core"`
	}
	if err := json.Unmarshal(out.Result, &st); err != nil {
		t.Fatal(err)
	}
	if !st.Running || st.StartedAt == "" {
		t.Fatalf("start result: %+v", st)
	}
	if !strings.HasPrefix(st.Core, "v") {
		t.Fatalf("core version = %q", st.Core)
	}

	// Replace (core.start while running) onto node b: same inbound-less
	// config shape, different selected node.
	mustOK(t, base, "core.start", map[string]any{"session": startSession(t, nodes, nodes[1].ID)})
	if st := statusOf(t, base); st["running"] != true {
		t.Fatalf("after replace: %v", st)
	}

	// Replace with an invalid session must keep the running instance up.
	_, out = call(t, base, testToken, "core.start", map[string]any{
		"session": map[string]any{"mode": "no-such-mode"},
	})
	if out.Error == nil {
		t.Fatal("bad session: want rpc error")
	}
	if st := statusOf(t, base); st["running"] != true {
		t.Fatalf("bad replace took the instance down: %v", st)
	}

	// stop
	out = mustOK(t, base, "core.stop", map[string]any{})
	if err := json.Unmarshal(out.Result, &st); err != nil {
		t.Fatal(err)
	}
	if st.Running {
		t.Fatalf("stop result still running: %+v", st)
	}
	if st := statusOf(t, base); st["running"] != false {
		t.Fatalf("after stop: %v", st)
	}
}

func TestConfigDry(t *testing.T) {
	base := startTestServer(t)
	out := mustOK(t, base, "config.dry", map[string]any{
		"session": map[string]any{
			"mode":     "global",
			"inbound":  map[string]any{"listen": "127.0.0.1", "port": 0, "type": "mixed"},
			"entries":  []any{},
			"selected": "",
		},
	})
	var cfg map[string]any
	if err := json.Unmarshal(out.Result, &cfg); err != nil {
		t.Fatal(err)
	}
	if cfg["outbounds"] == nil {
		t.Fatalf("config.dry result missing outbounds: %v", cfg)
	}
}

func TestSSEEvents(t *testing.T) {
	base := startTestServer(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+"/events", nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+testToken)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	events := make(chan string, 8)
	go func() {
		sc := bufio.NewScanner(resp.Body)
		for sc.Scan() {
			events <- sc.Text()
		}
	}()

	seen := func(want string) {
		t.Helper()
		for {
			select {
			case line := <-events:
				if line == want {
					return
				}
			case <-ctx.Done():
				t.Fatalf("timeout waiting for SSE %q", want)
			}
		}
	}

	nodes := parseNodes(t, "anytls://e0c664d9-415f-30b5-aeaf-547be3870274@127.0.0.1:26019/#ev")
	mustOK(t, base, "core.start", map[string]any{"session": startSession(t, nodes, nodes[0].ID)})
	seen("event: core.started")
	mustOK(t, base, "core.stop", map[string]any{})
	seen("event: core.stopped")
}
