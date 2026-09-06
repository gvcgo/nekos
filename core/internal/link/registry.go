package link

import (
	"strings"

	"nekos/core/internal/model"
)

// knownSchemes that we recognise but do not parse yet are reported as
// "not implemented" rather than "unsupported" so the roadmap state is
// visible to the user.
var knownSchemes = map[string]bool{
	"vmess": true, "vless": true, "ss": true, "ssr": true,
	"hysteria2": true, "hy2": true, "tuic": true, "wireguard": true,
	"naive": true, "socks": true, "http": true, "hysteria": true,
	"happ": true, "nekobox": true, "v2raytun": true,
}

var parserByScheme = map[string]func(string) (*model.Node, error){
	"anytls": parseAnyTLS,
	"trojan": parseTrojan,
	"vless":  parseVless,
}

// parseOne parses a single raw link into a node, reporting errors in a
// format suited to batch import.
func parseOne(raw string, lineNo int, result *model.ImportResult, seen map[string]bool) {
	raw = strings.TrimSpace(raw)
	idx := strings.Index(raw, "://")
	if idx <= 0 {
		result.Errors = append(result.Errors, model.ImportError{Line: lineNo, Snippet: snippet(raw), Reason: "no scheme"})
		return
	}
	scheme := raw[:idx]
	parser, ok := parserByScheme[scheme]
	if !ok {
		reason := "unsupported scheme"
		if knownSchemes[scheme] {
			reason = "scheme not implemented yet (P1)"
		}
		result.Errors = append(result.Errors, model.ImportError{Line: lineNo, Snippet: snippet(raw), Reason: reason + ": " + scheme})
		return
	}
	node, err := parser(raw)
	if err != nil {
		result.Errors = append(result.Errors, model.ImportError{Line: lineNo, Snippet: snippet(raw), Reason: err.Error()})
		return
	}
	if seen[node.ID] {
		return // de-duplicate by content hash
	}
	seen[node.ID] = true
	result.Nodes = append(result.Nodes, node)
}
