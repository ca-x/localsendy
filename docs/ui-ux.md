# UI and UX direction

The primary information architecture follows official LocalSend: Send, Receive, and Settings. Large screens use a stable left rail; screens below 768px use a labeled three-item bottom navigation.

## Design decisions

- Flat, privacy-oriented interface with one blue primary action and amber file accents.
- Cards use restrained 7–8px radii and visible borders instead of decorative glass or deep shadows.
- Nearby devices are always explicit selectable buttons; state is communicated by icon, border, and text, not color alone.
- File picking supports both drag-and-drop and a standard file input.
- Async operations disable repeated submission and display status feedback.
- High-frequency navigation has no entrance choreography. Press feedback uses a 120–160ms transform transition.
- Motion is limited to loading rotation and small state transitions; `prefers-reduced-motion` removes them.
- All touch targets are at least 44px, focus rings remain visible, and mobile content reserves space for the fixed bottom navigation.

The generated baseline is stored in `design-system/localsendy/MASTER.md`; implementation-specific decisions in this document take precedence where the generated landing-page defaults do not fit an application workspace.
