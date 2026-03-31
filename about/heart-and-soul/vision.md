# Vision

We are not building a dashboard.

We are building a presence engine: a local runtime that gives LLMs a way to occupy space, hold state, stream media, react live, and participate in the user's environment as something more than a chat box.

## Why this exists

Today, LLMs are mostly trapped in three forms:

- the CLI
- the chat transcript
- the generated app or webpage

All three are useful, and all three are incomplete.

A CLI gives a model command execution, but almost no ambient or visual presence. A chat UI gives a model conversation, but weak spatial state and weak persistence. A generated webpage gives a model an artifact, but not a live, shared environment.

What is missing is a way for an LLM to have presence:

- to hold a region of a screen over time
- to update it continuously
- to stream audio or video into it
- to react to touch, buttons, pointer, voice, or sensors
- to synchronize captions, highlights, overlays, and interaction against a shared clock
- to yield control to the human without friction

This project exists to create that substrate.

## Core thesis

The goal is not to let LLMs "make UIs."

The goal is to let LLMs become live participants on a screen.

That means the system must treat the following as first-class concepts:

- space
- time
- media
- interaction
- ownership
- revocation
- synchronization
- performance

The result should feel less like "a chatbot that can display things" and more like "an agent that can inhabit, manage, and negotiate a live visual environment."

## Visual identity is modular

The HUD's visual appearance — colors, typography, outlines, backdrops, component styling — must never be hardcoded in the runtime. Every visual element is a **component** that separates contract from implementation:

- The runtime defines what a component *does* (a subtitle occupies the bottom of the screen, renders text readably over arbitrary backgrounds).
- An author defines what a component *looks like* (white text with black outline over a 60% opacity backdrop, using 28px system sans-serif).
- An operator selects which author's implementation is active — swapping visual identity without changing agent code or runtime behavior.

This is the same principle as zone publishing (agents declare intent, runtime decides rendering) applied to visual identity itself. If two people disagree about what subtitles should look like, they author different component profiles and the operator chooses. The runtime ships sensible defaults, but every default is overridable.

This extensibility is not a post-v1 aspiration. It is load-bearing for v1: the design token system, component type contracts, and component profile format are specified and implemented alongside the core rendering pipeline. Making visual components fully and easily extensible is part of the product.

## Performance is part of the product

For this system, performance is not an optimization pass. It is part of the meaning of the product.

If touch is delayed, the agent is not interactive. If word-highlighting drifts from speech, the agent is not present. If a live video tile stutters while the dashboard updates, the system is not trustworthy. If chatty state updates overwhelm the renderer, the screen stops being an instrument and becomes theater.

Low latency, high throughput, timing precision, synchronization, backpressure, and graceful degradation are not secondary engineering concerns. They are the foundation.

## Non-goals

These are things tze_hud is explicitly not, and must not drift toward:

**Not a window manager.** The runtime composits agent content within its own surface. It does not manage OS windows, virtual desktops, or application lifecycles. It is one surface (fullscreen or overlay), not a shell for other applications.

**Not a browser shell.** Browser surfaces may eventually exist as a node type within tiles, but the compositor is not built on web technology and does not aspire to become one. The DOM is not the rendering model. HTML is not the content format. The browser is a guest, not the host.

**Not a remote desktop.** The system runs locally on the display node. It is not a thin client, not a VNC/RDP surface, not a cloud-rendered stream. Remote agents connect to the local runtime; the runtime owns the pixels locally.

**Not a notification engine.** Agents can show overlays and interruptions, but the system's purpose is sustained presence, not alert delivery. If the primary experience becomes a stream of notifications, the product has failed. Quiet hours, interruption classes, and attention governance (see privacy.md) exist to prevent this.

**Not a general-purpose UI framework.** The system provides a fixed set of node types, a fixed scene model, and a fixed composition pipeline. It is not a toolkit for building arbitrary applications. Agents work within the scene model's constraints; they do not extend the renderer.

**Not a chatbot with a screen.** If the dominant interaction pattern is "user types, agent responds, text scrolls," the product has failed to deliver on its thesis. The point is spatial, temporal, media-rich presence — not a fancier chat window.

## One-sentence definition

This project is a local, high-performance, agent-native display runtime that gives LLMs safe, synchronized, live, interactive presence on real screens — from wall displays to smart glasses.
