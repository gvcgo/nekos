// Package rpc implements the JSON-RPC 2.0 control surface between the
// orchestrator (Rust/Tauri) and the long-lived nekos-core control process
// (architecture.md §6.1/§6.2). The surface is a thin layer over the
// build/run/urltest/link packages: every method maps 1:1 onto a function
// those packages already expose, so protocol drift stays bounded.
//
// Transport: HTTP over 127.0.0.1 only; every request carries
// `Authorization: Bearer <token>`. POST /rpc carries JSON-RPC 2.0
// request/response messages; GET /events is an SSE stream of control-plane
// events (core.started / core.stopped today). Instance switching is the
// in-process rebuild path (run.Manager.Replace), matching §6.3.
package rpc

import (
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"sync"
	"time"

	qrcode "github.com/skip2/go-qrcode"

	"nekos/core/internal/build"
	"nekos/core/internal/link"
	"nekos/core/internal/run"
	"nekos/core/internal/urltest"
)

// JSON-RPC error codes (subset of the spec plus the application range).
const (
	codeInvalidRequest = -32600
	codeMethodNotFound = -32601
	codeInvalidParams  = -32602
	codeApplication    = -32000
)

// Server is the JSON-RPC control surface for one Manager.
type Server struct {
	mgr   *run.Manager
	token string
	hub   *eventHub
	http  *http.Server
	ln    net.Listener

	urlRunsMu sync.Mutex
	urlRuns   map[string]*urlTestRun // in-flight batch probes (progress polls)
}

// NewServer returns a Server authenticated by token. token must be
// non-empty; the orchestrator generates it and passes it over argv/stdin.
func NewServer(token string, mgr *run.Manager) *Server {
	return &Server{
		token:   token,
		mgr:     mgr,
		hub:     newEventHub(),
		urlRuns: make(map[string]*urlTestRun),
	}
}

// Token returns the bearer token the server requires.
func (s *Server) Token() string { return s.token }

// Serve blocks serving requests on ln. Returns nil after Close.
func (s *Server) Serve(ln net.Listener) error {
	s.ln = ln
	s.http = &http.Server{Handler: http.HandlerFunc(s.route)}
	err := s.http.Serve(ln)
	if errors.Is(err, http.ErrServerClosed) {
		return nil
	}
	return err
}

// Close shuts the HTTP server down (the Manager is owned by the caller).
func (s *Server) Close() error {
	if s.http != nil {
		return s.http.Close()
	}
	if s.ln != nil {
		return s.ln.Close()
	}
	return nil
}

// Addr returns the bound listener address (nil before Serve).
func (s *Server) Addr() net.Addr {
	if s.ln != nil {
		return s.ln.Addr()
	}
	return nil
}

func (s *Server) route(w http.ResponseWriter, r *http.Request) {
	switch {
	case r.Method == http.MethodPost && r.URL.Path == "/rpc":
		s.handleRPC(w, r)
	case r.Method == http.MethodGet && r.URL.Path == "/events":
		s.handleEvents(w, r)
	default:
		http.NotFound(w, r)
	}
}

func (s *Server) authorized(r *http.Request) bool {
	const prefix = "Bearer "
	h := r.Header.Get("Authorization")
	if !strings.HasPrefix(h, prefix) {
		return false
	}
	return subtle.ConstantTimeCompare([]byte(strings.TrimPrefix(h, prefix)), []byte(s.token)) == 1
}

// ---- JSON-RPC dispatch ---------------------------------------------------

type rpcRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
}

type rpcError struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

func (e *rpcError) Error() string { return e.Message }

func errAppf(format string, a ...any) *rpcError {
	return &rpcError{Code: codeApplication, Message: fmt.Sprintf(format, a...)}
}

func (s *Server) handleRPC(w http.ResponseWriter, r *http.Request) {
	if !s.authorized(r) {
		writeError(w, http.StatusUnauthorized, nil, &rpcError{Code: codeApplication, Message: "unauthorized"})
		return
	}
	body, err := io.ReadAll(io.LimitReader(r.Body, 1<<20))
	if err != nil {
		writeError(w, http.StatusBadRequest, nil, &rpcError{Code: codeInvalidRequest, Message: "read body: " + err.Error()})
		return
	}
	var req rpcRequest
	if err := json.Unmarshal(body, &req); err != nil {
		writeError(w, http.StatusBadRequest, nil, &rpcError{Code: codeInvalidRequest, Message: "malformed JSON"})
		return
	}
	if req.JSONRPC != "2.0" {
		writeError(w, http.StatusBadRequest, req.ID, &rpcError{Code: codeInvalidRequest, Message: "jsonrpc must be 2.0"})
		return
	}
	if len(req.Params) == 0 || string(req.Params) == "null" {
		req.Params = json.RawMessage("{}")
	}
	result, rpcErr := s.dispatch(req.Method, req.Params)
	if rpcErr != nil {
		writeError(w, http.StatusOK, req.ID, rpcErr)
		return
	}
	writeResult(w, req.ID, result)
}

func writeResult(w http.ResponseWriter, id json.RawMessage, result any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"jsonrpc": "2.0",
		"id":      idOrNull(id),
		"result":  result,
	})
}

func writeError(w http.ResponseWriter, status int, id json.RawMessage, rpcErr *rpcError) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(map[string]any{
		"jsonrpc": "2.0",
		"id":      idOrNull(id),
		"error":   rpcErr,
	})
}

func idOrNull(id json.RawMessage) any {
	if len(id) == 0 {
		return nil
	}
	var v any
	if err := json.Unmarshal(id, &v); err != nil {
		return nil
	}
	return v
}

func (s *Server) dispatch(method string, params json.RawMessage) (any, *rpcError) {
	switch method {
	case "parse.text":
		return s.methodParse(params)
	case "config.dry":
		return s.methodConfigDry(params)
	case "core.start":
		return s.methodStart(params)
	case "core.stop":
		return s.methodStop(params)
	case "core.status":
		return s.methodStatus(params)
	case "core.url_test":
		return s.methodURLTest(params)
	case "core.url_test_progress":
		return s.methodURLTestProgress(params)
	case "encode":
		return s.methodEncode(params)
	case "qr":
		return s.methodQR(params)
	default:
		return nil, &rpcError{Code: codeMethodNotFound, Message: "unknown method " + method}
	}
}

// decodeParams decodes a {"key": ...} params object into out.
func decodeParams(params json.RawMessage, out any) *rpcError {
	if err := json.Unmarshal(params, out); err != nil {
		return &rpcError{Code: codeInvalidParams, Message: "invalid params: " + err.Error()}
	}
	return nil
}

// sessionFromParams decodes the {"session": ...} payload shared by every
// session-taking method and applies the CLI defaults.
func sessionFromParams(params json.RawMessage) (*build.Session, *rpcError) {
	var p struct {
		Session json.RawMessage `json:"session"`
	}
	if rpcErr := decodeParams(params, &p); rpcErr != nil {
		return nil, rpcErr
	}
	if len(p.Session) == 0 {
		return nil, &rpcError{Code: codeInvalidParams, Message: "missing session"}
	}
	var sess build.Session
	if err := json.Unmarshal(p.Session, &sess); err != nil {
		return nil, &rpcError{Code: codeInvalidParams, Message: "invalid session: " + err.Error()}
	}
	if sess.Mode == "" {
		sess.Mode = "global"
	}
	if sess.LogLevel == "" {
		sess.LogLevel = "warn"
	}
	return &sess, nil
}

// ---- methods -------------------------------------------------------------

func (s *Server) methodParse(params json.RawMessage) (any, *rpcError) {
	var p struct {
		Text string `json:"text"`
	}
	if rpcErr := decodeParams(params, &p); rpcErr != nil {
		return nil, rpcErr
	}
	if p.Text == "" {
		return nil, errAppf("text must not be empty")
	}
	return link.Parse(p.Text), nil
}

func (s *Server) methodConfigDry(params json.RawMessage) (any, *rpcError) {
	sess, rpcErr := sessionFromParams(params)
	if rpcErr != nil {
		return nil, rpcErr
	}
	raw, err := build.AssembleJSON(sess)
	if err != nil {
		return nil, errAppf("%v", err)
	}
	return json.RawMessage(raw), nil
}

func (s *Server) methodStart(params json.RawMessage) (any, *rpcError) {
	sess, rpcErr := sessionFromParams(params)
	if rpcErr != nil {
		return nil, rpcErr
	}
	var err error
	if s.mgr.Running() {
		// Rebuild path: switch node / mode / route in-process.
		err = s.mgr.Replace(sess)
	} else {
		err = s.mgr.Start(sess)
	}
	if err != nil {
		return nil, errAppf("%v", err)
	}
	st := s.status()
	s.hub.publish("core.started", st)
	return st, nil
}

func (s *Server) methodStop(params json.RawMessage) (any, *rpcError) {
	if err := s.mgr.Stop(); err != nil {
		return nil, errAppf("stop core: %v", err)
	}
	st := s.status()
	s.hub.publish("core.stopped", st)
	return st, nil
}

func (s *Server) methodStatus(params json.RawMessage) (any, *rpcError) {
	return s.status(), nil
}

func (s *Server) methodURLTest(params json.RawMessage) (any, *rpcError) {
	var sess urltest.Session
	if rpcErr := decodeParams(params, &sess); rpcErr != nil {
		return nil, rpcErr
	}
	var p struct {
		RunID string `json:"run_id"`
	}
	if rpcErr := decodeParams(params, &p); rpcErr != nil {
		return nil, rpcErr
	}
	if p.RunID == "" {
		results, err := urltest.Probe(&sess)
		if err != nil {
			return nil, errAppf("%v", err)
		}
		return results, nil
	}
	// Tracked run: publish each finished node into the progress store so
	// the orchestrator can render results as they arrive
	// (core.url_test_progress); the response still carries the final array.
	run := s.startURLTestRun(p.RunID)
	defer s.finishURLTestRun(run)
	results, err := urltest.ProbeWithSink(&sess, func(r urltest.Result) {
		s.appendURLTestResult(run, r)
	})
	if err != nil {
		return nil, errAppf("%v", err)
	}
	return results, nil
}

// urlTestRun accumulates one batch's finished nodes for progress polling.
type urlTestRun struct {
	mu   sync.Mutex
	rows []urltest.Result
	done bool
}

func (s *Server) startURLTestRun(id string) *urlTestRun {
	s.urlRunsMu.Lock()
	defer s.urlRunsMu.Unlock()
	// Prune abandoned runs (never polled to completion) so a long-lived
	// daemon cannot accumulate them.
	for len(s.urlRuns) >= 32 {
		var oldest string
		for rid := range s.urlRuns {
			oldest = rid
			break
		}
		delete(s.urlRuns, oldest)
	}
	run := &urlTestRun{}
	s.urlRuns[id] = run
	return run
}

func (s *Server) appendURLTestResult(run *urlTestRun, r urltest.Result) {
	run.mu.Lock()
	defer run.mu.Unlock()
	run.rows = append(run.rows, r)
}

func (s *Server) finishURLTestRun(run *urlTestRun) {
	run.mu.Lock()
	run.done = true
	run.mu.Unlock()
}

func (s *Server) methodURLTestProgress(params json.RawMessage) (any, *rpcError) {
	var p struct {
		RunID string `json:"run_id"`
	}
	if rpcErr := decodeParams(params, &p); rpcErr != nil {
		return nil, rpcErr
	}
	s.urlRunsMu.Lock()
	run, ok := s.urlRuns[p.RunID]
	s.urlRunsMu.Unlock()
	if !ok {
		return map[string]any{"found": false}, nil
	}
	run.mu.Lock()
	defer run.mu.Unlock()
	// Collapse duplicate ids (a retried node re-reports after its first
	// error) keeping each id's final value, in first-seen order.
	merged := make([]urltest.Result, 0, len(run.rows))
	at := make(map[string]int, len(run.rows))
	for _, r := range run.rows {
		if i, ok := at[r.ID]; ok {
			merged[i] = r
			continue
		}
		at[r.ID] = len(merged)
		merged = append(merged, r)
	}
	done := run.done
	if done {
		// Hand the caller the final snapshot, then drop the run.
		delete(s.urlRuns, p.RunID)
	}
	return map[string]any{"found": true, "done": done, "results": merged}, nil
}

func (s *Server) methodEncode(params json.RawMessage) (any, *rpcError) {
	var p struct {
		Remark string          `json:"remark"`
		Out    json.RawMessage `json:"out"`
	}
	if rpcErr := decodeParams(params, &p); rpcErr != nil {
		return nil, rpcErr
	}
	linkStr, err := link.Encode(p.Remark, p.Out)
	if err != nil {
		return nil, errAppf("%v", err)
	}
	return linkStr, nil
}

func (s *Server) methodQR(params json.RawMessage) (any, *rpcError) {
	var p struct {
		Remark string          `json:"remark"`
		Out    json.RawMessage `json:"out"`
		Size   int             `json:"size"`
	}
	if rpcErr := decodeParams(params, &p); rpcErr != nil {
		return nil, rpcErr
	}
	linkStr, err := link.Encode(p.Remark, p.Out)
	if err != nil {
		return nil, errAppf("%v", err)
	}
	size := p.Size
	if size < 128 || size > 2048 {
		size = 300
	}
	png, err := qrcode.Encode(linkStr, qrcode.Medium, size)
	if err != nil {
		return nil, errAppf("qr encode: %v", err)
	}
	return base64.StdEncoding.EncodeToString(png), nil
}

// Status is the canonical control-plane status object.
type Status struct {
	Running   bool    `json:"running"`
	StartedAt *string `json:"started_at,omitempty"` // RFC3339, present when running
	Core      string  `json:"core"`                 // embedded sing-box version
}

func (s *Server) status() Status {
	snap := s.mgr.Snapshot()
	st := Status{Running: snap.Running, Core: snap.Core}
	if snap.Running {
		t := snap.StartedAt.Format(time.RFC3339)
		st.StartedAt = &t
	}
	return st
}

// ---- SSE events ----------------------------------------------------------

type eventHub struct {
	mu   sync.Mutex
	subs map[chan []byte]struct{}
}

func newEventHub() *eventHub {
	return &eventHub{subs: make(map[chan []byte]struct{})}
}

func (h *eventHub) subscribe() chan []byte {
	ch := make(chan []byte, 16)
	h.mu.Lock()
	h.subs[ch] = struct{}{}
	h.mu.Unlock()
	return ch
}

func (h *eventHub) unsubscribe(ch chan []byte) {
	h.mu.Lock()
	delete(h.subs, ch)
	h.mu.Unlock()
}

// publish broadcasts one SSE message ("event: <name>\ndata: <json>\n\n")
// to every subscriber, dropping for slow consumers.
func (h *eventHub) publish(name string, data any) {
	payload, err := json.Marshal(data)
	if err != nil {
		return
	}
	msg := append([]byte("event: "+name+"\ndata: "), payload...)
	msg = append(msg, '\n', '\n')
	h.mu.Lock()
	defer h.mu.Unlock()
	for ch := range h.subs {
		select {
		case ch <- msg:
		default:
		}
	}
}

func (s *Server) handleEvents(w http.ResponseWriter, r *http.Request) {
	if !s.authorized(r) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.WriteHeader(http.StatusOK)
	flusher.Flush() // deliver the headers now; the client blocks on first event
	ch := s.hub.subscribe()
	defer s.hub.unsubscribe(ch)
	for {
		select {
		case msg := <-ch:
			if _, err := w.Write(msg); err != nil {
				return
			}
			flusher.Flush()
		case <-r.Context().Done():
			return
		}
	}
}
