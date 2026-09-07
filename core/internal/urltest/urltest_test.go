package urltest

import "testing"

func TestIsTransient(t *testing.T) {
	transient := []string{
		`Head "http://www.gstatic.com/generate_204": context deadline exceeded`,
		"failed to create session: context deadline exceeded",
		"failed to create session: (dial tcp 54.251.177.157:26024: i/o timeout | dial tcp 18.136.208.182:26024: i/o timeout)",
		`Get "http://www.gstatic.com/generate_204": use of closed network connection`,
		"connection reset by peer",
		"tls handshake timeout",
	}
	for _, e := range transient {
		if !isTransient(e) {
			t.Errorf("isTransient(%q) = false, want true", e)
		}
	}
	deterministic := []string{
		"unexpected HTTP response status: 301",
		"unexpected HTTP response status: 403",
		"lookup freeyx.cloudflare88.eu.org: empty result",
		"lookup cf.zerone-cdn.pp.ua: (exchange6: NXDOMAIN | exchange4: NXDOMAIN)",
		"proxy handshake failed: EOF",
		"outbound missing",
	}
	for _, e := range deterministic {
		if isTransient(e) {
			t.Errorf("isTransient(%q) = true, want false", e)
		}
	}
}
