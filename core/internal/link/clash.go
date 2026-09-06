package link

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"

	"gopkg.in/yaml.v3"

	"nekos/core/internal/model"
)

func shortHash(b []byte) string {
	sum := sha1.Sum(b)
	return hex.EncodeToString(sum[:8])
}

// parseClashPayload converts the `proxies:` section of a Clash/Mihomo
// subscription YAML into sing-box outbound nodes. Groups and rules are
// intentionally ignored (server-list import, matching v2rayN/nekobox
// behavior for Clash subscriptions).
func parseClashPayload(text string) *model.ImportResult {
	result := &model.ImportResult{Nodes: []*model.Node{}, Errors: []model.ImportError{}}
	seen := make(map[string]bool)

	root := map[string]any{}
	if err := yaml.Unmarshal([]byte(text), &root); err != nil {
		result.Errors = append(result.Errors, model.ImportError{
			Line: 0, Snippet: snippet(text), Reason: "clash yaml: " + err.Error(),
		})
		return result
	}
	proxiesVal, ok := root["proxies"]
	if !ok {
		result.Errors = append(result.Errors, model.ImportError{
			Line: 0, Snippet: snippet(text), Reason: "clash yaml: no 'proxies' section",
		})
		return result
	}
	proxies, ok := proxiesVal.([]any)
	if !ok {
		result.Errors = append(result.Errors, model.ImportError{
			Line: 0, Snippet: snippet(text), Reason: "clash yaml: 'proxies' is not a list",
		})
		return result
	}

	for _, proxyVal := range proxies {
		proxy := asStringMap(proxyVal)
		name := str(proxy, "name")
		if name == "" {
			continue
		}
		out, err := clashProxyToOutbound(proxy)
		if err != nil {
			result.Errors = append(result.Errors, model.ImportError{
				Line: 0, Snippet: name, Reason: err.Error(),
			})
			continue
		}
		payload, _ := json.Marshal(out)
		id := shortHash(payload)
		if seen[id] {
			continue
		}
		seen[id] = true
		result.Nodes = append(result.Nodes, &model.Node{ID: id, Remark: name, Out: payload})
	}
	return result
}

// clashProxyToOutbound maps one Clash proxy entry to a sing-box outbound
// options object. Unsupported entries return an error naming the reason.
func clashProxyToOutbound(p map[string]any) (map[string]any, error) {
	typ := str(p, "type")
	server := str(p, "server")
	port := intV(p, "port")
	name := str(p, "name")
	if server == "" || port == 0 {
		return nil, fmt.Errorf("clash %s %q: missing server/port", typ, name)
	}
	base := map[string]any{"server": server, "server_port": int(port)}

	switch typ {
	case "ss":
		password := str(p, "password")
		cipher := str(p, "cipher")
		if password == "" || cipher == "" {
			return nil, fmt.Errorf("clash ss %q: missing password/cipher", name)
		}
		if plugin := str(p, "plugin"); plugin != "" {
			return nil, fmt.Errorf("clash ss %q: plugin not supported yet", name)
		}
		base["type"] = "shadowsocks"
		base["method"] = cipher
		base["password"] = password
	case "anytls":
		password := str(p, "password")
		if password == "" {
			return nil, fmt.Errorf("clash anytls %q: missing password", name)
		}
		base["type"] = "anytls"
		base["password"] = password
		attachClashTLS(base, p, true)
	case "vmess":
		uuid := str(p, "uuid")
		if uuid == "" {
			return nil, fmt.Errorf("clash vmess %q: missing uuid", name)
		}
		base["type"] = "vmess"
		base["uuid"] = uuid
		base["alter_id"] = intV(p, "alterId")
		if sec := str(p, "security"); sec != "" && sec != "auto" {
			base["security"] = sec
		}
		if err := attachClashTLSTransport(base, p); err != nil {
			return nil, err
		}
	case "vless":
		uuid := str(p, "uuid")
		if uuid == "" {
			return nil, fmt.Errorf("clash vless %q: missing uuid", name)
		}
		base["type"] = "vless"
		base["uuid"] = uuid
		if flow := str(p, "flow"); flow != "" {
			base["flow"] = flow
		}
		if err := attachClashTLSTransport(base, p); err != nil {
			return nil, err
		}
	case "trojan":
		password := str(p, "password")
		if password == "" {
			return nil, fmt.Errorf("clash trojan %q: missing password", name)
		}
		base["type"] = "trojan"
		base["password"] = password
		if err := attachClashTLSTransport(base, p); err != nil {
			return nil, err
		}
	case "hysteria2", "hy2":
		base["type"] = "hysteria2"
		if password := str(p, "password"); password != "" {
			base["password"] = password
		}
		if obfs := str(p, "obfs"); obfs != "" {
			base["obfs"] = obfs
		}
		if obfsPwd := str(p, "obfs-password"); obfsPwd != "" {
			base["obfs_password"] = obfsPwd
		}
		attachClashTLS(base, p, false)
	case "tuic":
		base["type"] = "tuic"
		// Clash tuic uses uuid+password as auth pair.
		base["uuid"] = str(p, "uuid")
		if password := str(p, "password"); password != "" {
			base["password"] = password
		}
		if cc := str(p, "congestion-controller"); cc != "" {
			base["congestion_control"] = cc
		}
		if relay := str(p, "udp-relay-mode"); relay != "" {
			base["udp_relay_mode"] = relay
		}
		attachClashTLS(base, p, true)
	case "wireguard":
		base["type"] = "wireguard"
		base["local_address"] = splitCSV(str(p, "ip"))
		if pk := str(p, "private-key"); pk != "" {
			base["private_key"] = pk
		}
		if pub := str(p, "public-key"); pub != "" {
			base["peer_public_key"] = pub
		}
		if pre := str(p, "preshared-key"); pre != "" {
			base["pre_shared_key"] = pre
		}
	default:
		return nil, fmt.Errorf("clash type %q unsupported", typ)
	}
	return base, nil
}

// attachClashTLSTransport handles vmess/vless/trojan shared fields: tls,
// servername, skip-cert-verify, client-fingerprint, reality options,
// network transport (ws/http/grpc) and their sub-options.
func attachClashTLSTransport(out map[string]any, p map[string]any) error {
	attachClashTLS(out, p, false)

	network := str(p, "network")
	if network == "" || network == "tcp" {
		return nil
	}
	switch network {
	case "ws":
		ws := map[string]any{"type": "ws"}
		opts := asStringMap(p["ws-opts"])
		if path := str(opts, "path"); path != "" {
			ws["path"] = path // keep query/ed verbatim (URL early data)
		}
		if headers := asStringMap(opts["headers"]); headers != nil {
			if host := str(headers, "Host"); host != "" {
				ws["headers"] = map[string]any{"Host": host}
			}
		}
		out["transport"] = ws
	case "grpc":
		grpc := map[string]any{"type": "grpc"}
		opts := asStringMap(p["grpc-opts"])
		if serviceName := str(opts, "grpc-service-name"); serviceName != "" {
			grpc["service_name"] = serviceName
		}
		out["transport"] = grpc
	case "http":
		httpOpts := map[string]any{"type": "http"}
		opts := asStringMap(p["http-opts"])
		if path := str(opts, "path"); path != "" {
			httpOpts["path"] = path
		}
		if headers := asStringMap(opts["headers"]); headers != nil {
			if host := str(headers, "Host"); host != "" {
				httpOpts["host"] = host
			}
		}
		out["transport"] = httpOpts
	case "h2":
		out["transport"] = map[string]any{"type": "httpupgrade"}
	default:
		return fmt.Errorf("clash network %q unsupported", network)
	}
	return nil
}

// attachClashTLS adds the tls object when the clash entry enables tls or
// reality; forceTLS forces the block regardless (tuic is always TLS).
func attachClashTLS(out map[string]any, p map[string]any, forceTLS bool) {
	tlsEnabled := forceTLS || boolV(p, "tls")
	if !tlsEnabled {
		// hy2/tuic style: may use sni without explicit "tls" flag
		if str(p, "servername") == "" && str(p, "sni") == "" {
			return
		}
		tlsEnabled = true
	}
	tls := map[string]any{"enabled": true}
	sni := firstNonEmpty(str(p, "servername"), str(p, "sni"))
	if sni != "" {
		tls["server_name"] = sni
	}
	if boolV(p, "skip-cert-verify") {
		tls["insecure"] = true
	}
	if fp := str(p, "client-fingerprint"); fp != "" {
		tls["utls"] = map[string]any{"enabled": true, "fingerprint": fp}
	}
	if raw := p["alpn"]; raw != nil {
		if values := alpnList(raw); len(values) > 0 {
			tls["alpn"] = values
		}
	}
	if reality := asStringMap(p["reality-opts"]); reality != nil {
		r := map[string]any{"enabled": true}
		if pk := str(reality, "public-key"); pk != "" {
			r["public_key"] = pk
		}
		if sid := str(reality, "short-id"); sid != "" {
			r["short_id"] = sid
		}
		tls["reality"] = r
	}
	out["tls"] = tls
}

// ---- helpers (yaml v3 -> any conversions) ----

func asStringMap(v any) map[string]any {
	switch m := v.(type) {
	case map[string]any:
		return m
	case map[any]any:
		out := make(map[string]any, len(m))
		for k, val := range m {
			if key, ok := k.(string); ok {
				out[key] = val
			}
		}
		return out
	default:
		return nil
	}
}

func str(m map[string]any, key string) string {
	if m == nil {
		return ""
	}
	switch v := m[key].(type) {
	case string:
		return v
	case nil:
		return ""
	default:
		return fmt.Sprintf("%v", v)
	}
}

func boolV(m map[string]any, key string) bool {
	if m == nil {
		return false
	}
	switch v := m[key].(type) {
	case bool:
		return v
	case string:
		return v == "true"
	default:
		return false
	}
}

func intV(m map[string]any, key string) int {
	if m == nil {
		return 0
	}
	switch v := m[key].(type) {
	case int:
		return v
	case int64:
		return int(v)
	case uint64:
		return int(v)
	case float64:
		return int(v)
	case string:
		var n int
		_, _ = fmt.Sscanf(v, "%d", &n)
		return n
	default:
		return 0
	}
}

func splitCSV(s string) []string {
	if s == "" {
		return nil
	}
	parts := strings.Split(s, ",")
	for i := range parts {
		parts[i] = strings.TrimSpace(parts[i])
	}
	return parts
}

func alpnList(v any) []string {
	switch a := v.(type) {
	case string:
		if a == "" {
			return nil
		}
		return strings.Fields(a)
	case []any:
		out := make([]string, 0, len(a))
		for _, item := range a {
			if s, ok := item.(string); ok && s != "" {
				out = append(out, s)
			}
		}
		return out
	case []string:
		return a
	default:
		return nil
	}
}
