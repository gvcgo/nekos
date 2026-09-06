package link

import (
	"crypto/sha1"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/url"
	"strconv"
	"strings"

	"nekos/core/internal/model"
)

// baseInfo is the common anatomy of a share link:
// scheme://[user[:pass]@]host:port?query#fragment
type baseInfo struct {
	Scheme   string
	User     string
	Password string
	Host     string
	Port     uint16
	Query    url.Values
	Fragment string
}

func parseBase(raw string) (*baseInfo, error) {
	u, err := url.Parse(raw)
	if err != nil {
		return nil, fmt.Errorf("malformed URL: %w", err)
	}
	info := &baseInfo{
		Scheme:   u.Scheme,
		Query:    u.Query(),
		Fragment: u.Fragment,
	}
	if u.User != nil {
		info.User = u.User.Username()
		info.Password, _ = u.User.Password()
	}
	host, portStr, err := net.SplitHostPort(u.Host)
	if err != nil {
		return nil, fmt.Errorf("missing or invalid host:port: %w", err)
	}
	port, err := strconv.ParseUint(portStr, 10, 16)
	if err != nil {
		return nil, fmt.Errorf("invalid port %q", portStr)
	}
	info.Host = host
	info.Port = uint16(port)
	return info, nil
}

// boolParam interprets common 0/1/true/false query flags.
func boolParam(v url.Values, keys ...string) bool {
	for _, key := range keys {
		val := v.Get(key)
		switch strings.ToLower(val) {
		case "1", "true", "yes", "on":
			return true
		case "0", "false", "no", "off":
			return false
		}
	}
	return false
}

// makeNode builds a model.Node from a parsed share link. remark falls back
// to host:port when the fragment is empty.
func makeNode(raw string, remark string, out map[string]any) (*model.Node, error) {
	payload, err := json.Marshal(out)
	if err != nil {
		return nil, err
	}
	if remark == "" {
		if server, _ := out["server"].(string); server != "" {
			if port, ok := out["server_port"].(float64); ok {
				remark = server + ":" + strconv.FormatFloat(port, 'f', -1, 64)
			}
		} else {
			remark = "unknown"
		}
	}
	sum := sha1.Sum([]byte(raw))
	return &model.Node{
		ID:     hex.EncodeToString(sum[:8]),
		Remark: strings.TrimSpace(remark),
		Out:    payload,
	}, nil
}
