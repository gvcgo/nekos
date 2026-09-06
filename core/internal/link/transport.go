package link

import (
	"strings"
)

// Shared v2ray-transport mapping used by trojan/vless/vmess style links.

// websocketTransport maps ws params (path, host) to sing-box
// v2ray-transport websocket options.
//
// Empirical mapping note (2026-09, verified against live v2rayN-style
// servers): v2rayN encodes early data as an `ed` query appended to the
// path ("/?ed=2048") and the server side parses it from the request URL.
// sing-box's header-based variant (max_early_data) was rejected by the
// tested server (EOF at TLS handshake), while keeping the path verbatim
// connected end-to-end. Therefore ed is left in the path and
// max_early_data is intentionally not set.
func websocketTransport(b *baseInfo) (map[string]any, error) {
	path := b.Query.Get("path")
	if path == "" {
		path = "/"
	}
	transport := map[string]any{"type": "ws", "path": path}
	if host := b.Query.Get("host"); host != "" {
		transport["headers"] = map[string]any{"Host": host}
	}
	return transport, nil
}

func grpcTransport(b *baseInfo) map[string]any {
	return map[string]any{
		"type":         "grpc",
		"service_name": firstNonEmpty(b.Query.Get("serviceName"), b.Query.Get("service_name")),
	}
}

func httpupgradeTransport(b *baseInfo) (map[string]any, error) {
	path := b.Query.Get("path")
	if path == "" {
		path = "/"
	}
	transport := map[string]any{"type": "httpupgrade", "path": path}
	if host := b.Query.Get("host"); host != "" {
		transport["headers"] = map[string]any{"Host": host}
	}
	return transport, nil
}

// firstNonEmpty returns the first non-empty value.
func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if v != "" {
			return v
		}
	}
	return ""
}

func splitList(s string) []string {
	parts := strings.FieldsFunc(s, func(r rune) bool { return r == ',' || r == ' ' })
	if len(parts) == 0 {
		return nil
	}
	return parts
}
