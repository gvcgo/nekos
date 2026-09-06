package link

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net"
	"net/url"
	"strconv"
	"strings"
)

// Share-link encoder: reverses the parsers for the supported schemes so
// nodes can be exported ("copy link" / QR share). Only sing-box-native
// outbound JSON is expected as input.

type outMap = map[string]any

func encodeHostForURL(host string) string {
	if strings.Contains(host, ":") && !strings.HasPrefix(host, "[") {
		if net.ParseIP(host) != nil && strings.Contains(host, ":") {
			return "[" + host + "]"
		}
	}
	return host
}

func escFrag(s string) string {
	esc := url.QueryEscape(s)
	return strings.ReplaceAll(esc, "+", "%20")
}

func sub(o outMap, key string) string {
	if v, ok := o[key].(string); ok {
		return v
	}
	return ""
}

func subMap(o outMap, key string) outMap {
	if m, ok := o[key].(map[string]any); ok {
		return m
	}
	return nil
}

func boolSub(o outMap, key string) bool {
	if v, ok := o[key].(bool); ok {
		return v
	}
	return false
}

func queryParam(q url.Values, key, value string) {
	if value != "" {
		q.Set(key, value)
	}
}

func encodeTransportQuery(q url.Values, out outMap) {
	transport := subMap(out, "transport")
	if transport == nil {
		return
	}
	switch sub(transport, "type") {
	case "ws":
		q.Set("type", "ws")
		queryParam(q, "path", sub(transport, "path"))
		if headers := subMap(transport, "headers"); headers != nil {
			queryParam(q, "host", sub(headers, "Host"))
		}
	case "grpc":
		q.Set("type", "grpc")
		queryParam(q, "serviceName", sub(transport, "service_name"))
	case "httpupgrade":
		q.Set("type", "httpupgrade")
		queryParam(q, "path", sub(transport, "path"))
		if headers := subMap(transport, "headers"); headers != nil {
			queryParam(q, "host", sub(headers, "Host"))
		}
	}
}

func encodeTLSQuery(q url.Values, tls outMap, security string) {
	queryParam(q, "sni", sub(tls, "server_name"))
	if boolSub(tls, "insecure") {
		q.Set("allowInsecure", "1")
	}
	if utls := subMap(tls, "utls"); utls != nil {
		queryParam(q, "fp", sub(utls, "fingerprint"))
	}
	if security == "reality" {
		if reality := subMap(tls, "reality"); reality != nil {
			queryParam(q, "pbk", sub(reality, "public_key"))
			queryParam(q, "sid", sub(reality, "short_id"))
		}
	}
}

func anytlsLink(remark string, o outMap) (string, error) {
	host := encodeHostForURL(sub(o, "server"))
	base := fmt.Sprintf("anytls://%s@%s:%s", sub(o, "password"), host, num(o, "server_port"))
	q := url.Values{}
	if tls := subMap(o, "tls"); tls != nil {
		queryParam(q, "sni", sub(tls, "server_name"))
		if boolSub(tls, "insecure") {
			q.Set("insecure", "1")
		}
		if utls := subMap(tls, "utls"); utls != nil {
			queryParam(q, "fp", sub(utls, "fingerprint"))
		}
	}
	return base + queryAndFrag(q, remark), nil
}

func trojanLink(remark string, o outMap) (string, error) {
	host := encodeHostForURL(sub(o, "server"))
	base := fmt.Sprintf("trojan://%s@%s:%s", sub(o, "password"), host, num(o, "server_port"))
	q := url.Values{}
	if tls := subMap(o, "tls"); tls != nil {
		q.Set("security", "tls")
		encodeTLSQuery(q, tls, "tls")
	}
	encodeTransportQuery(q, o)
	return base + queryAndFrag(q, remark), nil
}

func vlessLink(remark string, o outMap) (string, error) {
	host := encodeHostForURL(sub(o, "server"))
	base := fmt.Sprintf("vless://%s@%s:%s", sub(o, "uuid"), host, num(o, "server_port"))
	q := url.Values{}
	q.Set("encryption", "none")
	queryParam(q, "flow", sub(o, "flow"))
	queryParam(q, "packetEncoding", sub(o, "packet_encoding"))
	if tls := subMap(o, "tls"); tls != nil {
		security := "tls"
		if reality := subMap(tls, "reality"); reality != nil {
			security = "reality"
		}
		q.Set("security", security)
		encodeTLSQuery(q, tls, security)
	}
	encodeTransportQuery(q, o)
	return base + queryAndFrag(q, remark), nil
}

func shadowsocksLink(remark string, o outMap) (string, error) {
	host := encodeHostForURL(sub(o, "server"))
	userinfo := base64.RawURLEncoding.EncodeToString(
		[]byte(sub(o, "method") + ":" + sub(o, "password")),
	)
	return fmt.Sprintf("ss://%s@%s:%s", userinfo, host, num(o, "server_port")) +
		"#" + escFrag(remark), nil
}

func queryAndFrag(q url.Values, remark string) string {
	var sb strings.Builder
	if enc := q.Encode(); enc != "" {
		sb.WriteString("?")
		sb.WriteString(enc)
	}
	if remark != "" {
		sb.WriteString("#")
		sb.WriteString(escFrag(remark))
	}
	return sb.String()
}

func num(o outMap, key string) string {
	switch v := o[key].(type) {
	case float64:
		return strconv.FormatInt(int64(v), 10)
	case int:
		return strconv.Itoa(v)
	default:
		return "0"
	}
}

// Encode converts one outbound options JSON (plus display name) into its
// share link. Unsupported outbound types return an error naming the type.
func Encode(remark string, raw json.RawMessage) (string, error) {
	var o outMap
	if err := json.Unmarshal(raw, &o); err != nil {
		return "", fmt.Errorf("invalid outbound json: %w", err)
	}
	switch sub(o, "type") {
	case "anytls":
		return anytlsLink(remark, o)
	case "trojan":
		return trojanLink(remark, o)
	case "vless":
		return vlessLink(remark, o)
	case "shadowsocks", "ss":
		return shadowsocksLink(remark, o)
	default:
		return "", fmt.Errorf("导出暂不支持 %q 类型节点", sub(o, "type"))
	}
}
