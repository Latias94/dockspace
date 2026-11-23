# Keep logical surfaces separate from native windows

A surface identifies docking content; a native window identifies an OS resource
with its own lifetime. An externally owned Root window may compose several
surface placements. A Child has one placement, which defaults to the whole
client area and can project a smaller region within custom window chrome.
Treating those identities as interchangeable would prevent shared-window hosts
and correlate close, capture and retirement with the wrong resource.

Placements publish as a complete atomic roster. Close and destruction act on the
physical window's full roster; hit, output and pane-focus authority remain exact
to each placement. Child creation retains durable source ownership until all
required fresh first-live outputs are presented. A change to the child region
requires fresh output for the same exact child binding. Preparation or visibility
alone cannot commit that transfer. Root continuation preserves the external
resource while replacing logical bindings, with fresh presentation required
afterward.

An operation captures every physical lifetime needed for completion, including
the destination of a presentation rehome. Destruction or replacement of that
destination cancels the transfer even before any preview output exists. Normal
presentation refresh does not replace the captured window lifetime. Source
content stays owned until the existing exact first-live protocol completes.

Native close preflight and final publication use the same candidate-workspace
validation, including surviving children whose recovery targets must remain.
A successful cancellation resumes the existing physical placements; it must
also restore input after fresh geometry and presentation. An exactly completed
planned close releases its old recovery obligation, while unplanned destruction
still requires recovery. A surface retained by close policy can subsequently be
registered with a fresh physical lifetime.

Accepted ingress can become stale because an earlier record retires its exact
binding. A known retired window's narrow, non-destruction close tail is consumed
without changing ownership. Duplicate or conflicting registrations produce an
individual rejection and allow the FIFO to continue. Unknown bindings and
contradictory protocol facts remain errors. A binding registered and replaced
within one frame must still be listed for exact producer cleanup; never require
a host acknowledgment for an identity the committed report cannot expose.

Native placement intent is separate from the actual pointer hover. Foreign
windows block covered docking targets even when an explicit edge-local intent
authorizes native creation. A core-issued drag identity correlates a host's
provisional resource with the committed create request; the affine request and
existing first-live saga retain their execution and content-transfer authority.

This adds explicit mapping and lifetime obligations to the host, but supports
multiple UI frameworks without embedding a window manager in the core. Missing
platform facts remain Unknown instead of being inferred from cached geometry.
See [platform support](../PLATFORM_SUPPORT.md).

Fatal cleanup with a reducible ingress prefix advances without UI output. If
every exact physical destruction has committed, every admitted create has
terminated and input/output owners have
settled, the adapter can derive an empty live-window inventory using its last
committed backend roster. It submits that inventory through normal snapshot
transfer and settlement with the existing prepaid storage. Handler detachment
does not prove destruction; missing lifetime or backend evidence retains the
cleanup obligation.

A permanently rejected immutable prefix cannot be repaired by later cleanup
facts or an empty inventory. The host freezes semantic admission and retains
that generation's FIFO and affine owners while its backend terminates all
physical and asynchronous work. Once those owners are truly terminal, the whole
generation may be disposed of as failed. This does not claim committed
quiescence or permit any old result to enter a fresh session. See the
[native failure contract](../../crates/dockspace/docs/guide.md#permanently-rejected-native-ingress).
