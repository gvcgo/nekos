package link

import (
	"fmt"

	"nekos/core/internal/model"
)

// parseAnyTLS parses an anytls:// share link into a sing-box anytls
// outbound options object.
//
// Grammar (per the common share-link dialect):
//
//	anytls://<uuid>@<server>:<port>/?sni=...&insecure=1&fp=...&password=...#<remark>
//
// The UUID is the protocol password. tls is always enabled for anytls.
func parseAnyTLS(raw string) (*model.Node, error) {
	b, err := parseBase(raw)
	if err != nil {
		return nil, err
	}
	password := b.Password
	if password == "" {
		password = b.Query.Get("password")
	}
	if password == "" {
		password = b.User
	}
	if password == "" {
		return nil, fmt.Errorf("anytls: missing password/uuid")
	}

	out := map[string]any{
		"type":        "anytls",
		"server":      b.Host,
		"server_port": int(b.Port),
		"password":    password,
	}
	tls := map[string]any{"enabled": true}
	if sni := b.Query.Get("sni"); sni != "" {
		tls["server_name"] = sni
	}
	if boolParam(b.Query, "insecure", "allowInsecure", "skip-cert-verify") {
		tls["insecure"] = true
	}
	if fp := b.Query.Get("fp"); fp != "" {
		tls["utls"] = map[string]any{"enabled": true, "fingerprint": fp}
	}
	out["tls"] = tls
	return makeNode(raw, b.Fragment, out)
}
