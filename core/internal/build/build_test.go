package build_test

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	"nekos/core/internal/build"
)

// ruleSession returns a rule-mode session (selected node n1) ready to
// assemble. Assets point at placeholder files: assembly only embeds their
// paths, it never reads them.
func ruleSession(t *testing.T) *build.Session {
	t.Helper()
	return &build.Session{
		Mode:     "rule",
		Inbound:  build.InboundConfig{Port: 2080},
		Selected: "n1",
		Entries: []build.Entry{
			{ID: "n1", Out: json.RawMessage(`{"type":"direct"}`)},
		},
		RuleAssets: &build.RuleAssets{GeoipCn: "/tmp/geoip-cn.srs", GeositeCn: "/tmp/geosite-cn.srs"},
		LogLevel:   "info",
	}
}

func decodeConfig(t *testing.T, raw []byte) map[string]any {
	t.Helper()
	var cfg map[string]any
	if err := json.Unmarshal(raw, &cfg); err != nil {
		t.Fatalf("decoding assembled config: %v", err)
	}
	return cfg
}

func routeOf(t *testing.T, cfg map[string]any) map[string]any {
	t.Helper()
	route, ok := cfg["route"].(map[string]any)
	if !ok {
		t.Fatalf("config has no route object: %v", cfg["route"])
	}
	return route
}

// No explicit Route: the built-in bypass-mainland behaviour is preserved.
func TestRuleModeBuiltinUnchanged(t *testing.T) {
	raw, err := build.AssembleJSON(ruleSession(t))
	if err != nil {
		t.Fatalf("assemble: %v", err)
	}
	cfg := decodeConfig(t, raw)
	route := routeOf(t, cfg)
	if route["final"] != "out-n1" {
		t.Fatalf("final = %v, want out-n1", route["final"])
	}
	rules := route["rules"].([]any)
	if len(rules) != 1 {
		t.Fatalf("rules = %d entries, want 1", len(rules))
	}
	r0 := rules[0].(map[string]any)
	if r0["outbound"] != "direct" {
		t.Fatalf("builtin rule outbound = %v, want direct", r0["outbound"])
	}
	if sets, _ := r0["rule_set"].([]any); len(sets) != 2 {
		t.Fatalf("builtin rule_set len = %d, want 2", len(sets))
	}
	defs, ok := route["rule_set"].([]any)
	if !ok || len(defs) != 2 {
		t.Fatalf("rule_set defs = %v, want 2 local files", route["rule_set"])
	}
	if _, ok := cfg["dns"]; !ok {
		t.Fatal("builtin profile must configure DNS")
	}
	// outbound tag of the proxy node is present
	found := false
	for _, ob := range cfg["outbounds"].([]any) {
		if ob.(map[string]any)["tag"] == "out-n1" {
			found = true
		}
	}
	if !found {
		t.Fatal("proxy node outbound missing")
	}
}

func TestRuleModeCustomRouteMapsOutbounds(t *testing.T) {
	sess := ruleSession(t)
	sess.Route = &build.RouteConfig{
		Final: "direct",
		Rules: []json.RawMessage{
			json.RawMessage(`{"domain_suffix":["example.com"],"outbound":"block"}`),
			json.RawMessage(`{"rule_set":["geosite-cn"],"outbound":"direct"}`),
			json.RawMessage(`{"ip_cidr":["1.2.3.0/24"],"outbound":"proxy","invert":true}`),
		},
	}
	raw, err := build.AssembleJSON(sess)
	if err != nil {
		t.Fatalf("assemble: %v", err)
	}
	cfg := decodeConfig(t, raw)
	route := routeOf(t, cfg)
	if route["final"] != "direct" {
		t.Fatalf("final = %v, want direct", route["final"])
	}
	rules := route["rules"].([]any)
	if len(rules) != 3 {
		t.Fatalf("rules = %d entries, want 3", len(rules))
	}
	want := []string{
		`{"domain_suffix":["example.com"],"outbound":"block"}`,
		`{"rule_set":["geosite-cn"],"outbound":"direct"}`,
		`{"ip_cidr":["1.2.3.0/24"],"outbound":"out-n1","invert":true}`,
	}
	for i, ws := range want {
		var e map[string]any
		if err := json.Unmarshal([]byte(ws), &e); err != nil {
			t.Fatalf("bad expectation %d: %v", i, err)
		}
		r := rules[i].(map[string]any)
		for k, v := range e {
			if !reflect.DeepEqual(r[k], v) {
				t.Fatalf("rule %d: %s = %v, want %v", i, k, r[k], v)
			}
		}
	}
	// rule_set defs only cover referenced sets (geosite-cn here)
	defs := route["rule_set"].([]any)
	if len(defs) != 1 {
		t.Fatalf("rule_set defs = %v, want only geosite-cn", defs)
	}
	if defs[0].(map[string]any)["tag"] != "geosite-cn" {
		t.Fatalf("rule_set def tag = %v", defs[0].(map[string]any)["tag"])
	}
	if _, ok := cfg["dns"]; !ok {
		t.Fatal("profile referencing geosite-cn must configure DNS")
	}
}

// A profile that never references CN sets must not add rule_set/DNS blocks.
func TestRuleModeCustomNoCNSets(t *testing.T) {
	sess := ruleSession(t)
	sess.Route = &build.RouteConfig{
		Final: "proxy",
		Rules: []json.RawMessage{
			json.RawMessage(`{"domain_keyword":["ads"],"outbound":"block"}`),
		},
	}
	raw, err := build.AssembleJSON(sess)
	if err != nil {
		t.Fatalf("assemble: %v", err)
	}
	cfg := decodeConfig(t, raw)
	route := routeOf(t, cfg)
	if route["final"] != "out-n1" {
		t.Fatalf("final = %v, want out-n1 (proxy)", route["final"])
	}
	if _, has := route["rule_set"]; has {
		t.Fatal("no CN rule_set referenced but rule_set defs present")
	}
	if _, has := cfg["dns"]; has {
		t.Fatal("no CN rule_set referenced but dns block present")
	}
	rules := route["rules"].([]any)
	r0 := rules[0].(map[string]any)
	if r0["outbound"] != "block" {
		t.Fatalf("rule outbound = %v, want block", r0["outbound"])
	}
}

func TestRuleModeRejectsUnknownRuleSet(t *testing.T) {
	sess := ruleSession(t)
	sess.Route = &build.RouteConfig{
		Final: "proxy",
		Rules: []json.RawMessage{
			json.RawMessage(`{"rule_set":["geoip-cn","geoip-custom"],"outbound":"direct"}`),
		},
	}
	_, err := build.AssembleJSON(sess)
	if err == nil || !strings.Contains(err.Error(), `unknown rule_set "geoip-custom"`) {
		t.Fatalf("err = %v, want unknown rule_set error", err)
	}
}

func TestRuleModeRejectsUnknownOutbound(t *testing.T) {
	sess := ruleSession(t)
	sess.Route = &build.RouteConfig{
		Final: "proxy",
		Rules: []json.RawMessage{
			json.RawMessage(`{"domain_suffix":["x.com"],"outbound":"reject"}`),
		},
	}
	_, err := build.AssembleJSON(sess)
	if err == nil || !strings.Contains(err.Error(), `unsupported route outbound "reject"`) {
		t.Fatalf("err = %v, want unsupported outbound error", err)
	}
}
