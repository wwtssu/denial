import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../models/denial_window.dart';
import '../state/display_layout.dart';
import '../state/shell_controller.dart';
import '../theme/shell_theme.dart';
import '../theme/tokens.dart';
import 'desktop_workspace.dart';

/// Shell-owned title bar for server-side-decorated windows.
///
/// Drawn by the shell above the client region; the client texture starts
/// below this strip (see [DesktopWindowPlacement.contentRect]). Buttons
/// drive the existing workspace/bridge operations, so no window-control
/// protocol was added for them.
class DesktopWindowTitleBar extends ConsumerWidget {
  const DesktopWindowTitleBar({
    super.key,
    required this.window,
    required this.title,
    required this.maximized,
  });

  final DenialWindow window;
  final String title;
  final bool maximized;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final radius = math.max(
      0.0,
      ShellTheme.of(context).windowRadius - DesktopMetrics.frameBorder,
    );
    // The whole title strip is the drag surface: the gesture and move cursor
    // cover the full 36px height, not just the text line. Window buttons stay
    // interactive through the gesture arena (tap wins on release, pan wins
    // once the pointer moves past the touch slop). A plain click on the
    // decoration activates the window — content clicks already activate via
    // the native route, but decoration hits route to the shell scene, so the
    // title bar owns the activation itself.
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () => _activate(ref),
      onPanStart: (_) => _beginMove(ref),
      onPanUpdate: (details) => _moveBy(ref, details.delta),
      onPanEnd: (_) => _endMove(ref),
      onPanCancel: () => _endMove(ref),
      child: MouseRegion(
        cursor: SystemMouseCursors.move,
        child: Container(
          height: DesktopMetrics.titleBarHeight,
          decoration: BoxDecoration(
            color: ShellColors.windowFrameSurface,
            borderRadius: BorderRadius.vertical(top: Radius.circular(radius)),
          ),
          child: Row(
            children: <Widget>[
              Expanded(
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 12),
                  child: Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      color: ShellColors.textPrimary,
                      fontSize: 13,
                      fontWeight: FontWeight.w500,
                    ),
                  ),
                ),
              ),
              _WindowButton(
                icon: Icons.remove,
                semanticLabel: 'Minimize',
                onTap: () => _minimize(ref),
              ),
              _WindowButton(
                icon: maximized ? Icons.filter_none : Icons.crop_square,
                semanticLabel: maximized ? 'Restore' : 'Maximize',
                onTap: () => _toggleMaximize(ref),
              ),
              _WindowButton(
                icon: Icons.close,
                semanticLabel: 'Close',
                destructive: true,
                onTap: () => _close(ref),
              ),
            ],
          ),
        ),
      ),
    );
  }

  void _activate(WidgetRef ref) {
    ref.read(desktopWorkspaceProvider.notifier).activate(window.objectId);
    ref.read(shellControllerProvider.notifier).focusWindow(window);
  }

  void _beginMove(WidgetRef ref) {
    ref.read(desktopWorkspaceProvider.notifier).beginMove(window.objectId);
  }

  void _moveBy(WidgetRef ref, Offset delta) {
    ref
        .read(desktopWorkspaceProvider.notifier)
        .moveBy(window.objectId, delta);
  }

  void _endMove(WidgetRef ref) {
    ref.read(desktopWorkspaceProvider.notifier).endMove(window.objectId);
  }

  void _minimize(WidgetRef ref) {
    ref.read(shellControllerProvider.notifier).focusWindow(window);
    ref.read(desktopWorkspaceProvider.notifier).minimize(window.objectId);
  }

  void _toggleMaximize(WidgetRef ref) {
    ref.read(shellControllerProvider.notifier).focusWindow(window);
    final bounds = _outputBounds(ref, window.objectId);
    ref
        .read(desktopWorkspaceProvider.notifier)
        .toggleMaximized(window.objectId, bounds: bounds);
  }

  void _close(WidgetRef ref) {
    ref.read(denialBridgeProvider).closeWindow(window);
  }

  Rect _outputBounds(WidgetRef ref, int objectId) {
    final workspace = ref.read(desktopWorkspaceProvider);
    final displayLayout = ref.read(displayLayoutProvider);
    final viewSize = workspace.viewSize.isEmpty
        ? displayLayout?.logicalSize ?? Size.zero
        : workspace.viewSize;
    final canvas = Offset.zero & viewSize;
    final placement = workspace.placements[objectId];
    final outputs = displayLayout?.outputs;
    if (placement == null || outputs == null || outputs.isEmpty) {
      return canvas;
    }
    for (final output in outputs) {
      if (output.monitorId == placement.monitorId) {
        return displayLayout!.workAreaOf(output).intersect(canvas);
      }
    }
    return canvas;
  }
}

class _WindowButton extends StatefulWidget {
  const _WindowButton({
    required this.icon,
    required this.semanticLabel,
    required this.onTap,
    this.destructive = false,
  });

  final IconData icon;
  final String semanticLabel;
  final VoidCallback onTap;
  final bool destructive;

  @override
  State<_WindowButton> createState() => _WindowButtonState();
}

class _WindowButtonState extends State<_WindowButton> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final active = _hovered && widget.destructive;
    final background = !_hovered
        ? Colors.transparent
        : widget.destructive
        ? const Color(0xffe5484d)
        : Colors.white.withValues(alpha: 0.08);
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: Semantics(
        button: true,
        label: widget.semanticLabel,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          child: Container(
            width: 42,
            height: double.infinity,
            color: background,
            child: Icon(
              widget.icon,
              size: 16,
              color: active ? Colors.white : ShellColors.textPrimary,
            ),
          ),
        ),
      ),
    );
  }
}
