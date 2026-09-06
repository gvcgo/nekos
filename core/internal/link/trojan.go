package link

import (
	"fmt"

	"nekos/core/internal/model"
)

// parseTrojan parses a trojan:// share link (v2rayN dialect) into a
// sing-box trojan outbound options object.
//
// Supported parameters: security=tls|reality|none, sni, fp, alpn,
// allowInsecure/insecure, type=tcp|ws|grpc|httpupgrade, path, host,
// serviceName/service_name, ed (v2rayN early data, converted to
// sing-box ws max_early_data).
func parseTrojan(raw string) (*model.Node, error) {
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
		return nil, fmt.Errorf("trojan: missing password")
	}

	out := map[string]any{
		"type":        "trojan",
		"server":      b.Host,
		"server_port": int(b.Port),
		"password":    password,
	}

	security := b.Query.Get("security")
	transportType := b.Query.Get("type")

	if security != "" && security != "none" {
		tls := map[string]any{"enabled": true}
		sni := firstNonEmpty(b.Query.Get("sni"), b.Query.Get("host"), b.Host)
		if sni != "" {
			tls["server_name"] = sni
		}
		if boolParam(b.Query, "allowInsecure", "insecure", "skip-cert-verify") {
			tls["insecure"] = true
		}
		if fp := b.Query.Get("fp"); fp != "" {
			tls["utls"] = map[string]any{"enabled": true, "fingerprint": fp}
		}
		if alpn := b.Query.Get("alpn"); alpn != "" {
			tls["alpn"] = splitList(alpn)
		}
		out["tls"] = tls
	}

	switch transportType {
	case "", "tcp":
		// no transport
	case "ws":
		transport, err := websocketTransport(b)
		if err != nil {
			return nil, err
		}
		out["transport"] = transport
	case "grpc":
		out["transport"] = grpcTransport(b)
	case "httpupgrade", "http":
		transport, err := httpupgradeTransport(b)
		if err != nil {
			return nil, err
		}
		out["transport"] = transport
	default:
		return nil, fmt.Errorf("trojan: unsupported transport type %q", transportType)
	}
	return makeNode(raw, b.Fragment, out)
}
