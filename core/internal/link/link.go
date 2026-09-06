// Package link parses proxy share links and subscription payloads into
// sing-box outbound option objects. sing-box itself ships no share-link
// parser (verified against v1.14.0 module source), so this package owns the
// scheme grammars; every produced object is validated against sing-box
// option types by the build package at config-assembly time.
package link

import (
	"encoding/base64"
	"regexp"
	"strconv"
	"strings"

	"nekos/core/internal/model"
)

var schemePattern = regexp.MustCompile(`^([a-zA-Z0-9]+)://`)

// Parse splits raw subscription/link text into import results. It accepts:
// one link per line (CR/LF tolerant), whole-payload base64 (the common
// v2rayN/nekobox subscription encoding), and tolerates a trailing "url="
// wrapper line used by some providers.
func Parse(text string) *model.ImportResult {
	result := &model.ImportResult{Nodes: []*model.Node{}, Errors: []model.ImportError{}}
	seen := make(map[string]bool)
	lineNo := 0

	if looksLikeClash(text) {
		cr := parseClashPayload(text)
		result.Nodes = cr.Nodes
		result.Errors = cr.Errors
		return result
	}

	for _, rawLine := range strings.FieldsFunc(text, func(r rune) bool { return r == '\n' || r == '\r' }) {
		line := strings.TrimSpace(rawLine)
		if line == "" {
			continue
		}
		lineNo++
		// Provider wrappers:  url=.... or lines that are pure base64.
		candidates := []string{line}
		if strings.HasPrefix(line, "url=") {
			candidates = append(candidates, strings.TrimPrefix(line, "url="))
		}
		decoded := tryBase64Subscription(line)
		if decoded != "" {
			candidates = append(candidates, decoded)
		}
		matched := false
		for _, candidate := range candidates {
			for _, item := range splitPayload(candidate) {
				if schemePattern.MatchString(item) {
					matched = true
					parseOne(item, lineNo, result, seen)
				}
			}
		}
		if !matched && result != nil {
			result.Errors = append(result.Errors, model.ImportError{
				Line:    lineNo,
				Snippet: snippet(line),
				Reason:  "no supported link scheme found",
			})
		}
	}
	return result
}

// looksLikeClash detects Clash/Mihomo subscription payloads
// ("#!MANAGED-CONFIG" or a top-level "proxies:" key).
func looksLikeClash(text string) bool {
	if strings.Contains(text, "#!MANAGED-CONFIG") {
		return true
	}
	return regexp.MustCompile(`(?m)^\s*proxies:`).MatchString(text)
}

// ParseOne parses a single share link.
func ParseOne(text string) (*model.Node, error) {
	result := Parse(text)
	if len(result.Errors) > 0 {
		return nil, parseError(result.Errors[0])
	}
	if len(result.Nodes) == 0 {
		return nil, parseError(model.ImportError{Reason: "empty input"})
	}
	return result.Nodes[0], nil
}

// splitPayload breaks a base64 blob or a raw subscription body into link lines.
func splitPayload(body string) []string {
	body = strings.TrimSpace(body)
	var lines []string
	for _, line := range strings.FieldsFunc(body, func(r rune) bool { return r == '\n' || r == '\r' }) {
		line = strings.TrimSpace(line)
		if line != "" {
			lines = append(lines, line)
		}
	}
	if len(lines) > 0 {
		return lines
	}
	return []string{body}
}

// tryBase64Subscription decodes payloads that are entirely base64 and, once
// decoded, contain share links. Returns "" when the payload is not base64 or
// does not decode to link-bearing text.
func tryBase64Subscription(line string) string {
	if len(line) < 24 || !schemePattern.MatchString(line) {
		return ""
	}
	decoded, err := base64.RawURLEncoding.DecodeString(line)
	if err != nil {
		decoded, err = base64.StdEncoding.DecodeString(line)
	}
	if err != nil {
		return ""
	}
	out := string(decoded)
	if !strings.Contains(out, "://") {
		return ""
	}
	return out
}

func parseError(e model.ImportError) error {
	msg := e.Reason
	if e.Line > 0 {
		msg = e.Reason + " (line " + strconv.Itoa(e.Line) + ")"
	}
	return &ParseFailure{Message: msg}
}

func snippet(s string) string {
	if len(s) <= 120 {
		return s
	}
	return s[:120] + "..."
}

// ParseFailure is returned for unparseable links.
type ParseFailure struct{ Message string }

func (e *ParseFailure) Error() string { return e.Message }
