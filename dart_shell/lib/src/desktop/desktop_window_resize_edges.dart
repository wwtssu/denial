import 'package:flutter/widgets.dart';

import 'desktop_workspace.dart';

/// Resize-edge hover cursors for a window frame.
///
/// The visible border is a 1px hairline, far too thin to hover. Each side of
/// the frame exposes an ~8px band that switches the cursor to the matching
/// resize shape. The bands run the full height of the frame including the
/// title bar's top edge and side strips — the window's top border *is* the
/// title bar's top edge, so that strip participates in resize too. Only the
/// title bar's body (between the bands) stays a pure move surface (see
/// DesktopWindowTitleBar).
///
/// Cursors only — pointer events still reach the client below (MouseRegion
/// does not participate in gesture arbitration). The actual resize grab is
/// started by the compositor on an unmodified left press inside the border
/// band.
class DesktopWindowResizeEdges extends StatelessWidget {
  const DesktopWindowResizeEdges({super.key});

  static const double edgeWidth = 8.0;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = constraints.maxWidth;
        final height = constraints.maxHeight;
        final bottom = height - DesktopMetrics.frameBorder;
        return Stack(
          clipBehavior: Clip.none,
          children: <Widget>[
            // Sides (under the corners), full frame height: title bar side
            // strips included. The top band starts at the frame's top edge —
            // the window's top border is the title bar's top edge.
            _band(
              Rect.fromLTRB(edgeWidth, 0, width - edgeWidth, edgeWidth),
              SystemMouseCursors.resizeUpDown,
            ),
            _band(
              Rect.fromLTRB(edgeWidth, bottom - edgeWidth, width - edgeWidth, bottom),
              SystemMouseCursors.resizeUpDown,
            ),
            _band(
              Rect.fromLTRB(0, edgeWidth, edgeWidth, bottom - edgeWidth),
              SystemMouseCursors.resizeLeftRight,
            ),
            _band(
              Rect.fromLTRB(width - edgeWidth, edgeWidth, width, bottom - edgeWidth),
              SystemMouseCursors.resizeLeftRight,
            ),
            // Corners on top so they win the cursor. The top corners sit on
            // the title bar strip (the window's top border).
            _band(
              Rect.fromLTRB(0, 0, edgeWidth, edgeWidth),
              SystemMouseCursors.resizeUpLeftDownRight,
            ),
            _band(
              Rect.fromLTRB(width - edgeWidth, 0, width, edgeWidth),
              SystemMouseCursors.resizeUpRightDownLeft,
            ),
            _band(
              Rect.fromLTRB(0, bottom - edgeWidth, edgeWidth, bottom),
              SystemMouseCursors.resizeUpRightDownLeft,
            ),
            _band(
              Rect.fromLTRB(width - edgeWidth, bottom - edgeWidth, width, bottom),
              SystemMouseCursors.resizeUpLeftDownRight,
            ),
          ],
        );
      },
    );
  }

  Widget _band(Rect rect, MouseCursor cursor) {
    return Positioned.fromRect(
      rect: rect,
      child: MouseRegion(
        hitTestBehavior: HitTestBehavior.translucent,
        cursor: cursor,
        child: const SizedBox.expand(),
      ),
    );
  }
}
