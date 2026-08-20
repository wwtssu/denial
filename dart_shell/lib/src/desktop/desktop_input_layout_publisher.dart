import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../input/input_layout.dart';
import '../input/shell_interaction_registry.dart';
import '../models/denial_window.dart';
import '../state/desktop_window_switcher.dart';
import '../state/shell_controller.dart';
import 'desktop_workspace.dart';

class DesktopInputLayoutPublisher extends ConsumerStatefulWidget {
  const DesktopInputLayoutPublisher({required this.child, super.key});

  final Widget child;

  @override
  ConsumerState<DesktopInputLayoutPublisher> createState() =>
      _DesktopInputLayoutPublisherState();
}

class _DesktopInputLayoutPublisherState
    extends ConsumerState<DesktopInputLayoutPublisher> {
  final DesktopWindowConfigureTracker _configureTracker =
      DesktopWindowConfigureTracker();
  bool _scheduled = false;
  int _epoch = 0;
  InputLayoutSnapshot? _lastSnapshot;

  @override
  Widget build(BuildContext context) {
    ref.watch(
      shellControllerProvider.select(
        (state) => (state.windows, state.windowSnapshotSequence),
      ),
    );
    ref.watch(desktopWorkspaceProvider);
    ref.watch(desktopWindowSwitcherProvider);
    ref.watch(shellInteractionRegistryProvider);
    _schedulePublish(
      MediaQuery.sizeOf(context),
      MediaQuery.devicePixelRatioOf(context),
    );
    return widget.child;
  }

  void _schedulePublish(Size viewSize, double devicePixelRatio) {
    if (_scheduled) {
      return;
    }
    _scheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _scheduled = false;
      if (!mounted) {
        return;
      }

      final shell = ref.read(shellControllerProvider);
      final windows = shell.windows;
      ref
          .read(desktopWorkspaceProvider.notifier)
          .syncWindows(
            windows,
            viewSize,
            devicePixelRatio,
            snapshotSequence: shell.windowSnapshotSequence,
          );
      _publish(
        viewSize,
        ref.read(shellControllerProvider).windows,
        ref.read(desktopWorkspaceProvider),
        ref.read(shellInteractionRegistryProvider),
      );
    });
  }

  void _publish(
    Size viewSize,
    List<DenialWindow> windows,
    DesktopWorkspaceState desktop,
    ShellInteractionSnapshot interactions,
  ) {
    if (viewSize.width <= 0.0 || viewSize.height <= 0.0) {
      return;
    }

    final windowsById = <int, DenialWindow>{
      for (final window in windows)
        if (window.isUserApp) window.objectId: window,
    };
    final inputMethodPopups = windows
        .where((window) => window.isInputMethodPopup && window.geometry != null)
        .toList(growable: false);
    final switcher = ref.read(desktopWindowSwitcherProvider);
    final sampledSwitcherIds =
        interactions.capturesFullScene && (switcher?.isSelecting ?? false)
        ? switcher!.objectIds.toSet()
        : const <int>{};
    final placements =
        desktop.placements.values
            .where(
              (placement) =>
                  (!placement.minimized ||
                      desktop.isInOverview(placement.objectId) ||
                      sampledSwitcherIds.contains(placement.objectId)) &&
                  windowsById.containsKey(placement.objectId),
            )
            .toList(growable: false)
          ..sort((a, b) => compareDesktopWindowStack(a, b, windowsById));

    final canvas = Offset.zero & viewSize;
    var shellRegions = <Rect>[canvas];
    // Hover panels must not take pointer ownership of the whole scene. Changing
    // ownership while leaving a hot edge can synthesize another edge enter and
    // make the launcher repeatedly open and close over client windows.
    if (!interactions.capturesFullScene) {
      for (final popup in inputMethodPopups) {
        shellRegions = _subtractFromAll(shellRegions, popup.geometry!);
      }
      // The shell owns everything outside every window's frame: the gaps
      // between windows and the desktop. Server-side decoration (title bar)
      // belongs to its window as a shell-owned input region, so the full
      // frame is subtracted here.
      for (final placement in placements) {
        final window = windowsById[placement.objectId]!;
        shellRegions = _subtractFromAll(shellRegions, placement.frame);
        for (final popup in window.popupRoots) {
          final popupRect = window.mapSurfaceRect(popup, placement.contentRect);
          shellRegions = _subtractFromAll(shellRegions, popupRect);
        }
      }
    }
    for (final region in interactions.childRegions) {
      final clipped = region.intersect(canvas);
      if (!clipped.isEmpty) {
        shellRegions.add(clipped);
      }
    }

    final inputWindows = <InputWindowRegion>[];
    final visibleSurfaceIds = <int>{};
    for (final popup in inputMethodPopups) {
      visibleSurfaceIds.addAll(popup.visibleSurfaceIds);
      if (!interactions.capturesFullScene) {
        inputWindows.add(
          InputWindowRegion(
            window: popup,
            surfaceId: popup.objectId,
            rect: popup.geometry!,
            sourceRect: popup.contentCoordinateRect,
            z: 0x7fffffff,
            geometryLocked: true,
          ),
        );
      }
    }
    // Desktop widgets still sample their live main-surface textures. Keep
    // those surfaces presentation-visible without adding a client input
    // region or configuring the native window to the widget rectangle.
    for (final placement in desktop.placements.values) {
      if (!placement.minimized) {
        continue;
      }
      final window = windowsById[placement.objectId];
      if (window == null) {
        continue;
      }
      visibleSurfaceIds.addAll(window.mainVisibleSurfaceIds);
    }
    final zStride = placements.fold<int>(2, (stride, placement) {
      final layers = windowsById[placement.objectId]!.surfaceLayers.length + 2;
      return math.max(stride, layers);
    });
    final placementOrder = <int, int>{
      for (var index = 0; index < placements.length; index += 1)
        placements[index].objectId: index,
    };
    // The wire hit tester consumes the first matching window. Build this list
    // in its final topmost-first order so the codec normally needs neither a
    // defensive copy nor another sort.
    for (final placement in placements.reversed) {
      if (interactions.capturesFullScene) {
        final window = windowsById[placement.objectId]!;
        visibleSurfaceIds.addAll(window.visibleSurfaceIds);
        _configureWindowGeometry(
          window,
          placement.contentRect,
          nativeDragActive: placement.dragging,
        );
        continue;
      }
      final window = windowsById[placement.objectId]!;
      visibleSurfaceIds.addAll(window.visibleSurfaceIds);
      final baseZ = placementOrder[placement.objectId]! * zStride;
      // Server-side decoration: the shell-drawn title bar belongs to its
      // window as a decoration region. It is depth-tested in window order
      // but routes to the shell scene (the client has no buffer there).
      // CSD windows have no top decoration, so the list stays empty.
      final topDecoration = placement.contentRect.top - placement.frame.top;
      final decorations = topDecoration > 0.0
          ? <Rect>[
              Rect.fromLTRB(
                placement.frame.left,
                placement.frame.top,
                placement.frame.right,
                placement.contentRect.top,
              ),
            ]
          : const <Rect>[];
      final popupRoots = window.popupRoots.toList(growable: false).reversed;
      for (final popup in popupRoots) {
        final popupRect = window.mapSurfaceRect(popup, placement.contentRect);
        inputWindows.add(
          InputWindowRegion(
            window: window,
            surfaceId: popup.surfaceId,
            rect: popupRect,
            sourceRect: Rect.fromLTWH(
              0.0,
              0.0,
              popup.surfaceWidth,
              popup.surfaceHeight,
            ),
            z: baseZ + popup.compositionOrder + 1,
            geometryLocked: placement.fullscreen,
          ),
        );
      }
      inputWindows.add(
        InputWindowRegion(
          window: window,
          // A logical window region routes through the complete toplevel
          // surface tree. The primary texture may be a full-window child and
          // is a rendering choice, not an input target.
          surfaceId: window.objectId,
          // One geometry for the whole window: the frame (including the
          // shell-drawn title bar) is the input region. shellRegions subtract
          // the same frame, so the window owns its decoration and the border
          // band around the content. The parallel decorations list only
          // marks the title-bar strip for shell routing.
          rect: placement.frame,
          sourceRect: window.contentCoordinateRect,
          z: baseZ,
          geometryLocked: placement.fullscreen,
          decorations: decorations,
        ),
      );
      _configureWindowGeometry(
        window,
        placement.contentRect,
        nativeDragActive: placement.dragging,
      );
    }

    _configureTracker.retainWindowIds(windowsById.keys.toSet());
    final snapshot = InputLayoutSnapshot(
      epoch: _epoch + 1,
      shellRegions: shellRegions,
      windows: inputWindows,
      visibleSurfaceIds: visibleSurfaceIds.toList(growable: false),
      keyboardCapture: interactions.capturesKeyboard,
      exclusiveShellMode: interactions.compositorExclusive,
    );
    if (_lastSnapshot?.hasSameRoutingAs(snapshot) ?? false) {
      return;
    }

    if (!ref.read(denialBridgeProvider).publishInputLayout(snapshot)) {
      return;
    }
    ref.read(inputLayoutSnapshotProvider.notifier).publish(snapshot);
    _epoch = snapshot.epoch;
    _lastSnapshot = snapshot;
  }

  void _configureWindowGeometry(
    DenialWindow window,
    Rect contentRect, {
    required bool nativeDragActive,
  }) {
    final configuredGeometry = _configureTracker.update(
      window.objectId,
      contentRect,
      nativeDragActive: nativeDragActive,
    );
    if (configuredGeometry == null) {
      return;
    }
    ref.read(denialBridgeProvider).configureWindow(window, configuredGeometry);
  }
}

/// Tracks complete shell-authored window rectangles crossing the native
/// bridge. Location is part of the identity: dropping a position-only update
/// leaves Rust hit testing and Flutter composition on different coordinates.
class DesktopWindowConfigureTracker {
  final Map<int, ({int left, int top, int width, int height})> _configured =
      <int, ({int left, int top, int width, int height})>{};

  Rect? update(
    int objectId,
    Rect contentRect, {
    required bool nativeDragActive,
  }) {
    final geometry = (
      left: contentRect.left.round().clamp(0, 16384),
      top: contentRect.top.round().clamp(0, 16384),
      width: contentRect.width.round().clamp(64, 16384),
      height: contentRect.height.round().clamp(64, 16384),
    );
    final previous = _configured[objectId];
    if (previous == null) {
      // The native compositor owns initial placement and sizing. Seed from
      // the received geometry instead of echoing a newly discovered window.
      _configured[objectId] = geometry;
      return null;
    }
    if (nativeDragActive) {
      // Rust is the sole writer during a native move/resize grab. Do NOT
      // update _configured here: the value seen mid-grab is never sent, so
      // remembering it would make the post-grab equality check skip the
      // final configure (the "last seen == current" dedup bug).
      return null;
    }
    if (previous == geometry) {
      return null;
    }
    _configured[objectId] = geometry;
    return Rect.fromLTWH(
      geometry.left.toDouble(),
      geometry.top.toDouble(),
      geometry.width.toDouble(),
      geometry.height.toDouble(),
    );
  }

  void retainWindowIds(Set<int> activeObjectIds) {
    _configured.removeWhere(
      (objectId, _) => !activeObjectIds.contains(objectId),
    );
  }
}

List<Rect> _subtractFromAll(List<Rect> regions, Rect cut) {
  final result = <Rect>[];
  for (final region in regions) {
    result.addAll(_subtractRect(region, cut));
  }
  return result;
}

/// Splits [source] into the parts not covered by [cut].
List<Rect> _subtractRect(Rect source, Rect cut) {
  final overlap = source.intersect(cut);
  if (overlap.isEmpty) {
    return <Rect>[source];
  }

  final result = <Rect>[];
  void add(Rect rect) {
    if (rect.width > 0.0 && rect.height > 0.0) {
      result.add(rect);
    }
  }

  add(Rect.fromLTRB(source.left, source.top, source.right, overlap.top));
  add(Rect.fromLTRB(source.left, overlap.bottom, source.right, source.bottom));
  add(Rect.fromLTRB(source.left, overlap.top, overlap.left, overlap.bottom));
  add(Rect.fromLTRB(overlap.right, overlap.top, source.right, overlap.bottom));
  return result;
}
