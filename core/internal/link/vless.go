package link

import (
	"fmt"

	"nekos/core/internal/model"
)

// parseVless parses a vless:// share link (v2rayN dialect) into a sing-box
// vless outbound options object.
//
// Supported parameters: security=tls|reality|none, sni, fp, pbk, sid,
// flow, type=tcp|ws|grpc|httpupgrade, path, host, serviceName,
// ed, packetEncoding, encryption(=none accepted).
func parseVless(raw string) (*model.Node, error) {
	b, err := parseBase(raw)
	if err != nil {
		return nil, err
	}
	uuid := b.User
	if uuid == "" {
		uuid = b.Query.Get("uuid")
	}
	if uuid == "" {
		return nil, fmt.Errorf("vless: missing uuid")
	}
	if enc := b.Query.Get("encryption"); enc != "" && enc != "none" {
		// vless encryption other than none has no sing-box equivalent;
		// accept-and-ignore keeps v2rayN exports importable.
	}

	out := map[string]any{
		"type":        "vless",
		"server":      b.Host,
		"server_port": int(b.Port),
		"uuid":        uuid,
	}
	if flow := b.Query.Get("flow"); flow != "" {
		out["flow"] = flow
	}
	if pe := firstNonEmpty(b.Query.Get("packetEncoding"), b.Query.Get("packet_encoding")); pe != "" {
		out["packet_encoding"] = pe
	}

	security := b.Query.Get("security")
	if security != "" && security != "none" {
		tls := map[string]any{"enabled": true}
		sni := firstNonEmpty(b.Query.Get("sni"), b.Query.Get("host"), b.Host)
		if sni != "" {
			tls["server_name"] = sni
		}
		if boolParam(b.Query, "allowInsecure", "insecure", "skip-cert-verify") {
			tls["insecure"] = true
		}
		if alpn := b.Query.Get("alpn"); alpn != "" {
			tls["alpn"] = splitList(alpn)
		}
		switch security {
		case "tls":
			if fp := b.Query.Get("fp"); fp != "" {
				tls["utls"] = map[string]any{"enabled": true, "fingerprint": fp}
			}
		case "reality":
			reality := map[string]any{"enabled": true}
			if pbk := b.Query.Get("pbk"); pbk != "" {
				reality["public_key"] = pbk
			}
			if sid := b.Query.Get("sid"); sid != "" {
				reality["short_id"] = sid
			}
			tls["reality"] = reality
		default:
			return nil, fmt.Errorf("vless: unsupported security %q", security)
		}
		out["tls"] = tls
	}

	switch transportType := b.Query.Get("type"); transportType {
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
		return nil, fmt.Errorf("vless: unsupported transport type %q", transportType)
	}
	return makeNode(raw, b.Fragment, out)
}
