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

// Session is the orchestrator->core contract for one running profile.
type Session struct {
	Mode     string        `json:"mode"` // global | direct | rule(P1)
	Inbound  InboundConfig `json:"inbound"`
	Entries  []Entry       `json:"entries"`
	Selected string        `json:"selected"` // entry id; empty => direct
	LogLevel string        `json:"log_level,omitempty"`
}

// TagFor derives a stable, unique sing-box outbound tag from an entry id.
func TagFor(id string) string {
	return "out-" + id
}

const (
	tagDirect = "direct"
	tagBlock  = "block"
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
	if sess.Mode == "global" && sess.Selected != "" {
		target := findEntry(sess, sess.Selected)
		if target == nil {
			return nil, fmt.Errorf("selected entry %q not found", sess.Selected)
		}
		out := map[string]any{}
		if err := json.Unmarshal(target.Out, &out); err != nil {
			return nil, fmt.Errorf("entry %q: %w", sess.Selected, err)
		}
		out["tag"] = TagFor(sess.Selected)
		cfg["outbounds"] = append(cfg["outbounds"].([]any), out)
		final = TagFor(sess.Selected)
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
	parsed["route"].(map[string]any)["final"] = final
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
