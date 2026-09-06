// Package model defines the neutral wire types exchanged between the
// orchestrator (Rust/Tauri) and the nekos core process, plus the parser
// import results. A Node's Out field is a sing-box outbound options JSON
// object ({"type": "...", ...fields}) — i.e. the schema is sing-box's own
// option types, so there is no drift to maintain.
package model

import "encoding/json"

// Node is one imported server entry.
type Node struct {
	// ID is a stable content-derived identifier used for de-duplication.
	ID string `json:"id"`
	// Remark is the display name (from the link fragment, subscription
	// naming rules, or user edits).
	Remark string `json:"remark"`
	// Out is the sing-box outbound options object (without a "tag").
	Out json.RawMessage `json:"out"`
}

// OutboundType extracts the "type" field from a Node's Out JSON.
func (n *Node) OutboundType() string {
	var probe struct {
		Type string `json:"type"`
	}
	if len(n.Out) == 0 {
		return ""
	}
	_ = json.Unmarshal(n.Out, &probe)
	return probe.Type
}

// ImportError reports a single failed item while parsing a batch.
type ImportError struct {
	Line   int    `json:"line"`
	Snippet string `json:"snippet"` // truncated source
	Reason string `json:"reason"`
}

// ImportResult is the outcome of parsing a link/subscription batch.
type ImportResult struct {
	Nodes  []*Node       `json:"nodes"`
	Errors []ImportError `json:"errors"`
}
