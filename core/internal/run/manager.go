// Package run owns the sing-box box instance lifecycle and the node
// latency measurement used by the orchestrator's "test" flow. It only
// relies on the stable public box API (box.New/Start/Close) and on
// configs produced by the build package.
package run

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	box "github.com/sagernet/sing-box"
	"github.com/sagernet/sing-box/option"
	singjson "github.com/sagernet/sing/common/json"

	"nekos/core/internal/build"
)

// Manager supervises one running sing-box instance.
type Manager struct {
	mu       sync.Mutex
	instance *box.Box
	cancel   context.CancelFunc
	started  time.Time
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

// Start assembles and boots a full sing-box instance for the session.
// Fails fast when the session config is invalid or a listener cannot bind.
func (m *Manager) Start(sess *build.Session) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.instance != nil {
		return errors.New("already running: stop first")
	}
	raw, err := build.AssembleJSON(sess)
	if err != nil {
		return err
	}
	opts, err := singjson.UnmarshalExtendedContext[option.Options](build.RegistryContext(), raw)
	if err != nil {
		return fmt.Errorf("decode session config: %w", err)
	}
	ctx, cancel := context.WithCancel(build.RegistryContext())
	instance, err := box.New(box.Options{Context: ctx, Options: opts})
	if err != nil {
		cancel()
		return fmt.Errorf("create core: %w", err)
	}
	if err := instance.Start(); err != nil {
		cancel()
		return fmt.Errorf("start core: %w", err)
	}
	m.instance = instance
	m.cancel = cancel
	m.started = time.Now()
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

// Snapshot describes a running/stopped instance for status reporting.
type Snapshot struct {
	Running   bool      `json:"running"`
	StartedAt time.Time `json:"started_at,omitempty"`
	Core      string    `json:"core"`
}
