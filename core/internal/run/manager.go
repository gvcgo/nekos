// Package run owns the sing-box box instance lifecycle and the node
// latency measurement used by the orchestrator's "test" flow. It only
// relies on the stable public box API (box.New/Start/Close) and on
// configs produced by the build package.
package run

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"runtime/debug"
	"sync"
	"time"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/option"
	singjson "github.com/sagernet/sing/common/json"

	"nekos/core/internal/build"
)

// CoreVersion returns the version of the embedded sing-box module
// ("unknown" when build info is unavailable).
func CoreVersion() string {
	bi, ok := debug.ReadBuildInfo()
	if !ok {
		return "unknown"
	}
	for _, dep := range bi.Deps {
		if dep.Path == "github.com/sagernet/sing-box" {
			return dep.Version
		}
	}
	return "unknown"
}

// Manager supervises one running sing-box instance.
type Manager struct {
	mu       sync.Mutex
	instance *box.Box
	cancel   context.CancelFunc
	started  time.Time
	last     *build.Session // session that produced the current/previous instance (rollback)
}

func NewManager() *Manager { return &Manager{} }

// Running reports whether an instance is currently up.
func (m *Manager) Running() bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.instance != nil
}

// StartedAt returns the start time of the running instance (zero when idle).
func (m *Manager) StartedAt() time.Time {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.started
}

// Snapshot describes the current instance for status reporting.
func (m *Manager) Snapshot() Snapshot {
	m.mu.Lock()
	defer m.mu.Unlock()
	s := Snapshot{Core: CoreVersion()}
	if m.instance != nil {
		s.Running = true
		s.StartedAt = m.started
	}
	return s
}

// Start assembles and boots a full sing-box instance for the session.
// Fails fast when the session config is invalid or a listener cannot bind.
func (m *Manager) Start(sess *build.Session) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.instance != nil {
		return errors.New("already running: stop first")
	}
	instance, cancel, err := m.boot(sess)
	if err != nil {
		return err
	}
	m.install(instance, cancel, cloneSession(sess))
	return nil
}

// Replace rebuilds the running instance for a new session in-process
// (architecture.md §6.3 "instance rebuild" path) without exiting the
// process. The new config is validated while the old instance keeps
// serving, then the old box is closed and the new one started; a Start
// failure rolls back to the previous session's box so the user is never
// left without a proxy. When idle, Replace behaves like Start.
func (m *Manager) Replace(sess *build.Session) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.instance == nil {
		instance, cancel, err := m.boot(sess)
		if err != nil {
			return err
		}
		m.install(instance, cancel, cloneSession(sess))
		return nil
	}
	// Reject bad configs up front while the current instance keeps
	// serving: this is the common failure (unknown mode, bad rule refs,
	// an entry that no longer decodes) and must not take the old one down.
	if _, err := m.validate(sess); err != nil {
		return err
	}
	old := m.instance
	oldCancel := m.cancel
	// Free the old inbound listeners (the new session usually rebinds the
	// same port), then boot the replacement.
	_ = old.Close()
	oldCancel()
	instance, cancel, err := m.boot(sess)
	if err != nil {
		cancel()
		if m.last != nil {
			rinstance, rcancel, rerr := m.boot(m.last)
			if rerr == nil {
				m.install(rinstance, rcancel, cloneSession(m.last))
				return fmt.Errorf("replace core: %w (rolled back to the previous session)", err)
			}
			return fmt.Errorf("replace core: %w (rollback failed: %v)", err, rerr)
		}
		return fmt.Errorf("replace core: %w", err)
	}
	m.install(instance, cancel, cloneSession(sess))
	return nil
}

// Stop shuts the running instance down (idempotent).
func (m *Manager) Stop() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.instance == nil {
		return nil
	}
	instance := m.instance
	cancel := m.cancel
	m.instance = nil
	m.cancel = nil
	err := instance.Close()
	cancel()
	return err
}

// validate decodes the assembled config against the typed sing-box options
// without starting anything.
func (m *Manager) validate(sess *build.Session) (json.RawMessage, error) {
	raw, err := build.AssembleJSON(sess)
	if err != nil {
		return nil, err
	}
	if _, err := singjson.UnmarshalExtendedContext[option.Options](build.RegistryContext(), raw); err != nil {
		return nil, fmt.Errorf("decode session config: %w", err)
	}
	return raw, nil
}

// boot assembles, validates and starts a fresh instance for sess. The
// caller must hold m.mu.
func (m *Manager) boot(sess *build.Session) (*box.Box, context.CancelFunc, error) {
	raw, err := m.validate(sess)
	if err != nil {
		return nil, nil, err
	}
	opts, err := singjson.UnmarshalExtendedContext[option.Options](build.RegistryContext(), raw)
	if err != nil {
		return nil, nil, fmt.Errorf("decode session config: %w", err)
	}
	// The sing-box CLI only disables ANSI colors when --disable-color is
	// passed; in-process we always feed a pipe/file, so force it off here
	// (option.LogOptions.DisableColor is json:"-" and cannot come from the
	// session config).
	if opts.Log == nil {
		opts.Log = &option.LogOptions{}
	}
	opts.Log.DisableColor = true
	ctx, cancel := context.WithCancel(build.RegistryContext())
	instance, err := box.New(box.Options{Context: ctx, Options: opts})
	if err != nil {
		cancel()
		return nil, nil, fmt.Errorf("create core: %w", err)
	}
	if err := instance.Start(); err != nil {
		cancel()
		return nil, nil, fmt.Errorf("start core: %w", err)
	}
	return instance, cancel, nil
}

// install records a successfully booted instance. The caller must hold m.mu.
func (m *Manager) install(instance *box.Box, cancel context.CancelFunc, last *build.Session) {
	m.instance = instance
	m.cancel = cancel
	m.started = time.Now()
	m.last = last
}

// cloneSession returns a deep copy of sess (its Entry/Rule payloads are
// json.RawMessage slices that callers may reuse or mutate).
func cloneSession(sess *build.Session) *build.Session {
	b, err := json.Marshal(sess)
	if err != nil {
		return sess
	}
	var c build.Session
	if err := json.Unmarshal(b, &c); err != nil {
		return sess
	}
	return &c
}

// Snapshot describes a running/stopped instance for status reporting.
type Snapshot struct {
	Running   bool      `json:"running"`
	StartedAt time.Time `json:"started_at,omitempty"`
	Core      string    `json:"core"`
}
