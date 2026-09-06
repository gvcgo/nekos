// Package build assembles a full sing-box option.Options from a neutral
// session description. Node outbound objects are sing-box-native JSON, so
// assembly is unmarshal-and-validate against sing-box option types; this is
// what keeps the generated config in lockstep with the embedded core.
package build

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/sagernet/sing-box/include"
	"github.com/sagernet/sing-box/option"
	singjson "github.com/sagernet/sing/common/json"
)

// RegistryContext returns a fully bootstrapped context: include.Context
// registers every adapter/options registry that extended JSON unmarshal and
// box.New require (same bootstrap the sing-box CLI uses).
func RegistryContext() context.Context {
	return include.Context(context.Background())
}

// InboundConfig describes the local listener(s).
type InboundConfig struct {
	Listen string `json:"listen,omitempty"` // default 127.0.0.1
	Port   uint16 `json:"port"`             // 0 => ephemeral (not yet allocated)
	Type   string `json:"type,omitempty"`   // mixed | socks | http, default mixed
}

// Entry pairs a stable node id with its sing-box outbound options JSON.
type Entry struct {
	ID  string          `json:"id"`
	Out json.RawMessage `json:"out"`
}

// RuleAssets points at locally cached sing-box binary rule-set files
// (CN geoip/geosite) used by rule mode's bypass-mainland routing.
type RuleAssets struct {
	GeoipCn   string `json:"geoip_cn,omitempty"`
	GeositeCn string `json:"geosite_cn,omitempty"`
}

// StrategyConfig tunes the auto urltest group for strategy mode.
type StrategyConfig struct {
	URL       string `json:"url,omitempty"`
	Interval  string `json:"interval,omitempty"`
	Tolerance int    `json:"tolerance,omitempty"` // ms, default 50
}

// Session is the orchestrator->core contract for one running profile.
type Session struct {
	Mode       string          `json:"mode"` // global | direct | rule | strategy
	Inbound    InboundConfig   `json:"inbound"`
	Entries    []Entry         `json:"entries"`
	Selected   string          `json:"selected"` // entry id; empty => direct
	LogLevel   string          `json:"log_level,omitempty"`
	RuleAssets *RuleAssets     `json:"rule_assets,omitempty"`
	Strategy   *StrategyConfig `json:"strategy,omitempty"`
}

// TagFor derives a stable, unique sing-box outbound tag from an entry id.
func TagFor(id string) string {
	return "out-" + id
}

const (
	tagDirect        = "direct"
	tagBlock         = "block"
	tagStrategyGroup = "strategy-group"
)

// Assemble converts a session into validated sing-box options JSON.
// It does not bind anything; validation happens at unmarshal time.
func AssembleJSON(sess *Session) (json.RawMessage, error) {
	if len(sess.Inbound.Listen) == 0 {
		sess.Inbound.Listen = "127.0.0.1"
	}
	if sess.Inbound.Type == "" {
		sess.Inbound.Type = "mixed"
	}
	cfg := map[string]any{
		"inbounds": []any{},
		"outbounds": []any{
			map[string]any{"type": "direct", "tag": tagDirect},
			map[string]any{"type": "block", "tag": tagBlock},
		},
		"route": map[string]any{"final": tagDirect},
	}
	if sess.LogLevel != "" {
		cfg["log"] = map[string]any{"level": sess.LogLevel}
	}

	if sess.Inbound.Port > 0 {
		cfg["inbounds"] = []any{
			map[string]any{
				"type":        sess.Inbound.Type,
				"tag":         "local-in",
				"listen":      sess.Inbound.Listen,
				"listen_port": sess.Inbound.Port,
			},
		}
	}

	final := tagDirect
	var selectedOut map[string]any
	if sess.Selected != "" && sess.Mode != "direct" && sess.Mode != "strategy" {
		target := findEntry(sess, sess.Selected)
		if target == nil {
			return nil, fmt.Errorf("selected entry %q not found", sess.Selected)
		}
		selectedOut = map[string]any{}
		if err := json.Unmarshal(target.Out, &selectedOut); err != nil {
			return nil, fmt.Errorf("entry %q: %w", sess.Selected, err)
		}
		selectedOut["tag"] = TagFor(sess.Selected)
	}

	switch sess.Mode {
	case "global":
		if selectedOut != nil {
			cfg["outbounds"] = append(cfg["outbounds"].([]any), selectedOut)
			final = TagFor(sess.Selected)
		}
	case "direct":
		// keep final=direct
	case "rule":
		if sess.RuleAssets == nil || sess.RuleAssets.GeoipCn == "" || sess.RuleAssets.GeositeCn == "" {
			return nil, fmt.Errorf("rule mode requires rule_assets (geoip_cn + geosite_cn)")
		}
		if selectedOut == nil {
			return nil, fmt.Errorf("rule mode requires a selected node")
		}
		cfg["outbounds"] = append(cfg["outbounds"].([]any), selectedOut)
		final = TagFor(sess.Selected)
		// Bypass-mainland: CN ip/domain go direct; everything else -> node.
		cfg["route"] = map[string]any{
			"final": final,
			"rules": []any{
				map[string]any{
					"rule_set": []string{"geoip-cn", "geosite-cn"},
					"outbound": tagDirect,
				},
			},
			"rule_set": []any{
				map[string]any{"type": "local", "tag": "geoip-cn", "format": "binary", "path": sess.RuleAssets.GeoipCn},
				map[string]any{"type": "local", "tag": "geosite-cn", "format": "binary", "path": sess.RuleAssets.GeositeCn},
			},
		}
		// DNS: CN domains resolve via the system (direct path); anything
		// unmatched is left as a hostname for the proxy exit to resolve.
		cfg["dns"] = map[string]any{
			"servers": []any{
				map[string]any{"type": "local", "tag": "dns-direct"},
			},
			"rules": []any{
				map[string]any{
					"rule_set": []string{"geosite-cn"},
					"server":   "dns-direct",
				},
			},
		}
	case "strategy":
		if sess.Strategy == nil {
			return nil, fmt.Errorf("strategy mode requires strategy config")
		}
		if len(sess.Entries) == 0 {
			return nil, fmt.Errorf("strategy mode requires member entries")
		}
		// every member becomes an outbound; the urltest group keeps
		// re-testing and pins the fastest healthy one, switching
		// automatically when the current pick fails (v2rayN combo:
		// lowest-latency with failover re-test).
		var members []string
		for _, e := range sess.Entries {
			out := map[string]any{}
			if err := json.Unmarshal(e.Out, &out); err != nil {
				return nil, fmt.Errorf("entry %q: %w", e.ID, err)
			}
			tag := TagFor(e.ID)
			out["tag"] = tag
			cfg["outbounds"] = append(cfg["outbounds"].([]any), out)
			members = append(members, tag)
		}
		url := sess.Strategy.URL
		if url == "" {
			url = "http://www.gstatic.com/generate_204"
		}
		interval := sess.Strategy.Interval
		if interval == "" {
			interval = "1m"
		}
		tolerance := sess.Strategy.Tolerance
		if tolerance <= 0 {
			tolerance = 50
		}
		cfg["outbounds"] = append(cfg["outbounds"].([]any), map[string]any{
			"type":      "urltest",
			"tag":       tagStrategyGroup,
			"outbounds": members,
			"url":       url,
			"interval":  interval,
			"tolerance": tolerance,
		})
		final = tagStrategyGroup
	default:
		return nil, fmt.Errorf("unknown mode %q", sess.Mode)
	}

	// Validate + normalize by decoding into typed sing-box options.
	raw, err := json.Marshal(cfg)
	if err != nil {
		return nil, err
	}
	if _, err := singjson.UnmarshalExtendedContext[option.Options](RegistryContext(), raw); err != nil {
		return nil, fmt.Errorf("invalid sing-box config: %w", err)
	}
	// Reflect the decided route.final back through the typed layer is not
	// possible without re-encode; so re-encode from raw (already validated).
	parsed := map[string]any{}
	_ = json.Unmarshal(raw, &parsed)
	if route, ok := parsed["route"].(map[string]any); ok {
		route["final"] = final
	}
	out, err := json.Marshal(parsed)
	if err != nil {
		return nil, err
	}
	return out, nil
}

func findEntry(sess *Session, id string) *Entry {
	for i := range sess.Entries {
		if sess.Entries[i].ID == id {
			return &sess.Entries[i]
		}
	}
	return nil
}
