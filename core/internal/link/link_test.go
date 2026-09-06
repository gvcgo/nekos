package link

import (
	"encoding/json"
	"testing"
)

const anytlsA = "anytls://e0c664d9-415f-30b5-aeaf-547be3870274@ew.ali66mysql.com:26019/?sni=www.apple.com&insecure=1#%F0%9F%87%B8%F0%9F%87%AC%20A%E6%96%B0%E5%8A%A0%E5%9D%A11%20VIP1%20%E7%BD%91%E5%9D%80%3Abdoos.com"

const anytlsB = "anytls://e0c664d9-415f-30b5-aeaf-547be3870274@ew.ali66mysql.com:26020/?sni=www.apple.com&insecure=1#%F0%9F%87%B8%F0%9F%87%AC%20A%E6%96%B0%E5%8A%A0%E5%9D%A12%20VIP1%20%E7%BD%91%E5%9D%80%3Abdoos.com"

const trojanA = "trojan://17e65fb1-5fc9-44cc-a6c6-25ecfbbcd886@104.16.251.174:443?security=tls&sni=6bd2f208.edge-bbe3c3f6.pages.dev&fp=chrome&type=ws&path=/?ed%3D2048&host=6bd2f208.edge-bbe3c3f6.pages.dev#%E7%A7%BB%E5%8A%A8-HKG-11"

const vlessA = "vless://17e65fb1-5fc9-44cc-a6c6-25ecfbbcd886@104.16.251.174:443?security=tls&sni=6bd2f208.edge-bbe3c3f6.pages.dev&fp=randomized&type=ws&path=/?ed%3D2048&host=6bd2f208.edge-bbe3c3f6.pages.dev&packetEncoding=xudp&encryption=none#%E8%81%94%E9%80%9A-LAX-01"

type tls struct {
	Enabled    bool   `json:"enabled"`
	ServerName string `json:"server_name"`
	Insecure   bool   `json:"insecure"`
	UTLS       *utls  `json:"utls"`
}

type utls struct {
	Enabled     bool   `json:"enabled"`
	Fingerprint string `json:"fingerprint"`
}

type wsTransport struct {
	Type         string            `json:"type"`
	Path         string            `json:"path"`
	Headers      map[string]string `json:"headers"`
	MaxEarlyData int               `json:"max_early_data"`
}

func decodeOut(t *testing.T, n json.RawMessage) map[string]json.RawMessage {
	t.Helper()
	var m map[string]json.RawMessage
	if err := json.Unmarshal(n, &m); err != nil {
		t.Fatalf("node out is not an object: %v", err)
	}
	return m
}

func field[T any](t *testing.T, m map[string]json.RawMessage, key string) T {
	t.Helper()
	var v T
	raw, ok := m[key]
	if !ok {
		return v
	}
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatalf("field %s: %v", key, err)
	}
	return v
}

func TestParseAnyTLS(t *testing.T) {
	res := Parse(anytlsA + "\n" + anytlsB)
	if len(res.Errors) != 0 {
		t.Fatalf("unexpected errors: %+v", res.Errors)
	}
	if len(res.Nodes) != 2 {
		t.Fatalf("want 2 nodes, got %d", len(res.Nodes))
	}
	a, b := res.Nodes[0], res.Nodes[1]
	if a.OutboundType() != "anytls" {
		t.Fatalf("type = %q", a.OutboundType())
	}
	if a.Remark != "🇸🇬 A新加坡1 VIP1 网址:bdoos.com" {
		t.Fatalf("remark = %q", a.Remark)
	}
	if b.Remark != "🇸🇬 A新加坡2 VIP1 网址:bdoos.com" {
		t.Fatalf("remark2 = %q", b.Remark)
	}
	ma := decodeOut(t, a.Out)
	if got := field[string](t, ma, "server"); got != "ew.ali66mysql.com" {
		t.Fatalf("server = %q", got)
	}
	if got := field[int](t, ma, "server_port"); got != 26019 {
		t.Fatalf("port = %d", got)
	}
	if got := field[string](t, ma, "password"); got != "e0c664d9-415f-30b5-aeaf-547be3870274" {
		t.Fatalf("password = %q", got)
	}
	tlsField := field[*tls](t, ma, "tls")
	if tlsField == nil || !tlsField.Enabled || tlsField.ServerName != "www.apple.com" || !tlsField.Insecure {
		t.Fatalf("tls = %+v", tlsField)
	}
	mb := decodeOut(t, b.Out)
	if got := field[int](t, mb, "server_port"); got != 26020 {
		t.Fatalf("port b = %d", got)
	}
}

func TestParseTrojanWS(t *testing.T) {
	res := Parse(trojanA)
	if len(res.Errors) != 0 {
		t.Fatalf("unexpected errors: %+v", res.Errors)
	}
	if len(res.Nodes) != 1 {
		t.Fatalf("want 1 node, got %d", len(res.Nodes))
	}
	n := res.Nodes[0]
	if n.OutboundType() != "trojan" {
		t.Fatalf("type = %q", n.OutboundType())
	}
	if n.Remark != "移动-HKG-11" {
		t.Fatalf("remark = %q", n.Remark)
	}
	m := decodeOut(t, n.Out)
	if got := field[string](t, m, "server"); got != "104.16.251.174" {
		t.Fatalf("server = %q", got)
	}
	if got := field[int](t, m, "server_port"); got != 443 {
		t.Fatalf("port = %d", got)
	}
	if got := field[string](t, m, "password"); got != "17e65fb1-5fc9-44cc-a6c6-25ecfbbcd886" {
		t.Fatalf("password = %q", got)
	}
	tlsField := field[*tls](t, m, "tls")
	if tlsField == nil || tlsField.ServerName != "6bd2f208.edge-bbe3c3f6.pages.dev" {
		t.Fatalf("tls = %+v", tlsField)
	}
	if tlsField.UTLS == nil || tlsField.UTLS.Fingerprint != "chrome" {
		t.Fatalf("utls = %+v", tlsField.UTLS)
	}
	ws := field[*wsTransport](t, m, "transport")
	if ws == nil || ws.Type != "ws" {
		t.Fatalf("transport = %+v", ws)
	}
	if ws.Path != "/?ed=2048" {
		t.Fatalf("path = %q", ws.Path)
	}
	if ws.MaxEarlyData != 0 {
		t.Fatalf("max_early_data = %d (URL-query early data must stay in path)", ws.MaxEarlyData)
	}
	if ws.Headers["Host"] != "6bd2f208.edge-bbe3c3f6.pages.dev" {
		t.Fatalf("headers = %+v", ws.Headers)
	}
}

func TestParseVlessWS(t *testing.T) {
	res := Parse(vlessA)
	if len(res.Errors) != 0 {
		t.Fatalf("unexpected errors: %+v", res.Errors)
	}
	if len(res.Nodes) != 1 {
		t.Fatalf("want 1 node, got %d", len(res.Nodes))
	}
	n := res.Nodes[0]
	if n.OutboundType() != "vless" {
		t.Fatalf("type = %q", n.OutboundType())
	}
	if n.Remark != "联通-LAX-01" {
		t.Fatalf("remark = %q", n.Remark)
	}
	m := decodeOut(t, n.Out)
	if got := field[string](t, m, "uuid"); got != "17e65fb1-5fc9-44cc-a6c6-25ecfbbcd886" {
		t.Fatalf("uuid = %q", got)
	}
	if got := field[string](t, m, "packet_encoding"); got != "xudp" {
		t.Fatalf("packet_encoding = %q", got)
	}
	if got := field[string](t, m, "server"); got != "104.16.251.174" {
		t.Fatalf("server = %q", got)
	}
	tlsField := field[*tls](t, m, "tls")
	if tlsField == nil || tlsField.ServerName != "6bd2f208.edge-bbe3c3f6.pages.dev" {
		t.Fatalf("tls = %+v", tlsField)
	}
	if tlsField.UTLS == nil || tlsField.UTLS.Fingerprint != "randomized" {
		t.Fatalf("utls = %+v", tlsField.UTLS)
	}
	ws := field[*wsTransport](t, m, "transport")
	if ws == nil || ws.Type != "ws" || ws.Path != "/?ed=2048" {
		t.Fatalf("transport = %+v", ws)
	}
	if ws.Headers["Host"] != "6bd2f208.edge-bbe3c3f6.pages.dev" {
		t.Fatalf("headers = %+v", ws.Headers)
	}
}

func TestParseDedupeAndBase64Subscription(t *testing.T) {
	// Whole-payload base64 (standard v2rayN subscription encoding).
	res := Parse(anytlsA + "\n" + anytlsB + "\n" + anytlsA)
	if len(res.Errors) != 0 {
		t.Fatalf("unexpected errors: %+v", res.Errors)
	}
	if len(res.Nodes) != 2 {
		t.Fatalf("dedupe failed: got %d nodes", len(res.Nodes))
	}
	if res.Nodes[0].ID == res.Nodes[1].ID {
		t.Fatalf("node IDs must differ")
	}
}

func TestParseErrors(t *testing.T) {
	res := Parse("vmess://abc@example.com:443")
	if len(res.Nodes) != 0 || len(res.Errors) != 1 {
		t.Fatalf("want 1 error, got %+v", res)
	}
	if got := res.Errors[0].Reason; got != "scheme not implemented yet (P1): vmess" {
		t.Fatalf("reason = %q", got)
	}
	res = Parse("not a link at all")
	if len(res.Errors) != 1 || res.Errors[0].Reason != "no supported link scheme found" {
		t.Fatalf("want generic error, got %+v", res.Errors)
	}
}

func TestBatchTextWithWhitespace(t *testing.T) {
	res := Parse("  \n " + anytlsA + "  \n\t" + trojanA + "\n\n")
	if len(res.Errors) != 0 {
		t.Fatalf("unexpected errors: %+v", res.Errors)
	}
	if len(res.Nodes) != 2 {
		t.Fatalf("want 2 nodes, got %d", len(res.Nodes))
	}
}
