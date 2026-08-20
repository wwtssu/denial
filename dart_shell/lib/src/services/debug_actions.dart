import 'dart:async';

import 'package:flutter/widgets.dart' show Offset, Rect;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../desktop/desktop_workspace.dart';
import '../input/input_layout.dart';
import '../launcher/controllers/home_grid_controller.dart';
import '../launcher/models/desktop_app.dart';
import '../launcher/models/home_grid_item.dart';
import '../models/denial_window.dart';
import '../state/clipboard_tray.dart';
import '../state/shell_controller.dart';

/// Invocation context handed to [DebugActionTarget.invoke].
class DebugActionContext {
  DebugActionContext(this.ref);

  final WidgetRef ref;
}

/// A debuggable UI target, addressed by [key] and operated with [actions].
///
/// This mirrors the GlobalKey model: any component — a widget `State` or a
/// controller-backed surface — exposes itself under a stable key and answers
/// named actions. The debug HTTP server only knows keys and actions; it
/// never grows per-feature routes.
abstract class DebugActionTarget {
  String get key;

  Set<String> get actions;

  /// Invokes [action] with [args]; returns a JSON-able result.
  ///
  /// Unknown actions must throw [DebugActionException].
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  );
}

class DebugActionException implements Exception {
  DebugActionException(this.message);

  final String message;

  @override
  String toString() => message;
}

/// Registry of debug action targets.
///
/// Widget `State`s that want to be debuggable can mix in
/// [DebugActionsMixin], which registers and unregisters the target with the
/// widget lifecycle; controller-backed surfaces register plain targets.
class DebugActionRegistry {
  final Map<String, DebugActionTarget> _targets =
      <String, DebugActionTarget>{};

  void register(DebugActionTarget target) {
    _targets[target.key] = target;
  }

  bool unregister(String key) => _targets.remove(key) != null;

  DebugActionTarget? targetFor(String key) => _targets[key];

  List<String> get keys => _targets.keys.toList()..sort();

  Set<String>? actionsFor(String key) => _targets[key]?.actions;
}

final debugActionsProvider = Provider<DebugActionRegistry>((ref) {
  final registry = DebugActionRegistry();
  registry
    ..register(const StateTarget())
    ..register(const LauncherTarget())
    ..register(const AppsTarget())
    ..register(const ShellActionTarget())
    ..register(const WatchTarget())
    ..register(const InputLayoutTarget())
    ..register(const WindowTarget());
  return registry;
});

/// Latest platform pointer position, fed by the compositor's cursor stream.
final cursorPositionProvider =
    NotifierProvider<CursorPositionController, Offset?>(
      CursorPositionController.new,
    );

class CursorPositionController extends Notifier<Offset?> {
  StreamSubscription<Offset>? _subscription;

  @override
  Offset? build() {
    _subscription = ref
        .read(denialBridgeProvider)
        .cursorPositions
        .listen((position) => state = position);
    ref.onDispose(() => _subscription?.cancel());
    return null;
  }
}

/// Widget-lifecycle registration helper.
///
/// A `ConsumerState` that mixes this in becomes a debug target under
/// [debugTargetKey] for as long as it is mounted; the registry entry is
/// removed on dispose. This is the GlobalKey-style path for widget-level UI
/// operation.
mixin DebugActionsMixin<T extends ConsumerStatefulWidget>
    on ConsumerState<T>
    implements DebugActionTarget {
  String get debugTargetKey;

  @override
  String get key => debugTargetKey;

  @override
  void initState() {
    super.initState();
    ref.read(debugActionsProvider).register(this);
  }

  @override
  void dispose() {
    ref.read(debugActionsProvider).unregister(debugTargetKey);
    super.dispose();
  }
}

class StateTarget implements DebugActionTarget {
  const StateTarget();

  @override
  String get key => 'state';

  @override
  Set<String> get actions => const {'get'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    if (action != 'get') {
      throw DebugActionException('unknown state action: $action');
    }
    final workspace = context.ref.read(desktopWorkspaceProvider);
    final grid = context.ref.read(homeGridControllerProvider);
    final apps = <Map<String, Object?>>[];
    final localApps = <Map<String, Object?>>[];
    final slots = switch (grid) {
      AsyncData(:final value) => value.slots,
      _ => const <HomeGridItem?>[],
    };
    for (final item in slots) {
      final app = item?.app;
      if (app != null) {
        apps.add(<String, Object?>{
          'gridId': item!.id,
          'id': app.id,
          'name': app.name,
          'exec': app.exec,
          'categories': app.categories,
        });
      }
      final local = item?.localApp;
      if (local != null) {
        localApps.add(<String, Object?>{'gridId': item!.id, 'id': local.id});
      }
    }
    return <String, Object?>{
      'launcherOpen': workspace.launcherOpen,
      'overviewActive': workspace.overviewActive,
      'apps': apps,
      'localApps': localApps,
    };
  }
}

class LauncherTarget implements DebugActionTarget {
  const LauncherTarget();

  @override
  String get key => 'launcher';

  @override
  Set<String> get actions => const {'open', 'close', 'toggle'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    final workspace = context.ref.read(desktopWorkspaceProvider);
    switch (action) {
      case 'open':
        context
            .ref
            .read(desktopWorkspaceProvider.notifier)
            .showPanel(DesktopPanel.launcher);
        break;
      case 'close':
        context.ref.read(desktopWorkspaceProvider.notifier).closePanels();
        break;
      case 'toggle':
        if (workspace.launcherOpen) {
          context.ref.read(desktopWorkspaceProvider.notifier).closePanels();
        } else {
          context
              .ref
              .read(desktopWorkspaceProvider.notifier)
              .showPanel(DesktopPanel.launcher);
        }
        break;
      default:
        throw DebugActionException('unknown launcher action: $action');
    }
    return <String, Object?>{'ok': true, 'action': action};
  }
}

class AppsTarget implements DebugActionTarget {
  const AppsTarget();

  @override
  String get key => 'apps';

  @override
  Set<String> get actions => const {'refresh', 'launch'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    switch (action) {
      case 'refresh':
        await context
            .ref
            .read(homeGridControllerProvider.notifier)
            .refreshDesktopApps(reason: 'debug-api');
        return <String, Object?>{'ok': true};
      case 'launch':
        final requested = (args['id'] as String?)?.trim();
        final app = requested == null ? null : _findApp(context, requested);
        if (app == null) {
          return <String, Object?>{
            'ok': false,
            'error': 'app not found',
            'id': requested,
          };
        }
        final launched = await context.ref.read(appLauncherProvider).launch(
          app,
        );
        return <String, Object?>{'ok': launched, 'id': app.id};
      default:
        throw DebugActionException('unknown apps action: $action');
    }
  }

  DesktopApp? _findApp(DebugActionContext context, String requested) {
    final slots = switch (context.ref.read(homeGridControllerProvider)) {
      AsyncData(:final value) => value.slots,
      _ => const <HomeGridItem?>[],
    };
    for (final item in slots) {
      final app = item?.app;
      if (app == null) {
        continue;
      }
      if (item!.id == requested ||
          app.id == requested ||
          item.id == 'app:$requested') {
        return app;
      }
    }
    return null;
  }
}

/// Dispatches the shell's own action vocabulary by name (the same actions
/// compositor keybindings send), mirroring the simple cases of the native
/// shell-action handler.
class ShellActionTarget implements DebugActionTarget {
  const ShellActionTarget();

  @override
  String get key => 'shell';

  @override
  Set<String> get actions => const {'applications', 'clipboard'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    final workspace = context.ref.read(desktopWorkspaceProvider);
    switch (action) {
      case 'applications':
        if (workspace.launcherOpen) {
          context.ref.read(desktopWorkspaceProvider.notifier).closePanels();
        } else {
          context
              .ref
              .read(desktopWorkspaceProvider.notifier)
              .showPanel(DesktopPanel.launcher);
        }
        break;
      case 'clipboard':
        context
            .ref
            .read(clipboardTrayProvider.notifier)
            .toggle(monitorId: null);
        break;
      default:
        throw DebugActionException('unknown shell action: $action');
    }
    return <String, Object?>{'ok': true, 'action': action};
  }
}

/// Diagnostics for the launcher's file-watch refresh path.
class WatchTarget implements DebugActionTarget {
  const WatchTarget();

  @override
  String get key => 'watch';

  @override
  Set<String> get actions => const {'status'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    if (action != 'status') {
      throw DebugActionException('unknown watch action: $action');
    }
    return context
        .ref
        .read(homeGridControllerProvider.notifier)
        .debugWatchStatus();
  }
}

/// Dumps the last published input layout snapshot (shell regions and window
/// input regions) so pointer routing can be diagnosed against real geometry.
class InputLayoutTarget implements DebugActionTarget {
  const InputLayoutTarget();

  @override
  String get key => 'input';

  @override
  Set<String> get actions => const {'layout', 'cursor'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    if (action == 'cursor') {
      final position = context.ref.read(cursorPositionProvider);
      if (position == null) {
        return <String, Object?>{'known': false};
      }
      return <String, Object?>{
        'known': true,
        'x': position.dx.round(),
        'y': position.dy.round(),
      };
    }
    if (action != 'layout') {
      throw DebugActionException('unknown input action: $action');
    }
    final snapshot = context.ref.read(inputLayoutSnapshotProvider);
    if (snapshot == null) {
      return <String, Object?>{'published': false};
    }
    return <String, Object?>{
      'published': true,
      'epoch': snapshot.epoch,
      'shellRegions': [
        for (final region in snapshot.shellRegions) _rectJson(region),
      ],
      'windows': [
        for (final region in snapshot.windows)
          <String, Object?>{
            'windowId': region.window.objectId,
            'appId': region.window.appId,
            'rect': _rectJson(region.rect),
            'sourceRect': _rectJson(region.sourceRect),
            'z': region.z,
            'surfaceId': region.targetSurfaceId,
            'visible': region.visible,
            'hitTest': region.hitTest,
            'decorations': [
              for (final decoration in region.decorations) _rectJson(decoration),
            ],
          },
      ],
    };
  }
}

class WindowTarget implements DebugActionTarget {
  const WindowTarget();

  @override
  String get key => 'window';

  @override
  Set<String> get actions => const {'center', 'geometry', 'close'};

  @override
  Future<Map<String, Object?>> invoke(
    DebugActionContext context,
    String action,
    Map<String, Object?> args,
  ) async {
    final windowId = args['windowId'] is int ? args['windowId'] as int : null;
    final window = _selectWindow(context, windowId);
    if (window == null) {
      throw DebugActionException('no resizable user window available');
    }
    final workspace = context.ref.read(desktopWorkspaceProvider);
    final placement = workspace.placements[window.objectId];
    if (placement == null) {
      throw DebugActionException('window has no placement');
    }
    switch (action) {
      case 'center':
        final viewSize = workspace.viewSize;
        if (viewSize.isEmpty) {
          throw DebugActionException('workspace has no view size');
        }
        final frame = placement.frame;
        final target = Rect.fromLTWH(
          (viewSize.width - frame.width) / 2,
          (viewSize.height - frame.height) / 2,
          frame.width,
          frame.height,
        );
        final delta = target.topLeft - frame.topLeft;
        context
            .ref
            .read(desktopWorkspaceProvider.notifier)
            .moveBy(window.objectId, delta);
        final after =
            context
                .ref
                .read(desktopWorkspaceProvider)
                .placements[window.objectId]
                ?.frame;
        return <String, Object?>{
          'ok': true,
          'windowId': window.objectId,
          'delta': <String, Object?>{
            'x': delta.dx.round(),
            'y': delta.dy.round(),
          },
          'rect': _rectJson(after ?? frame),
        };
      case 'geometry':
        return <String, Object?>{
          'windowId': window.objectId,
          'rect': _rectJson(placement.frame),
        };
      case 'close':
        context.ref.read(denialBridgeProvider).closeWindow(window);
        return <String, Object?>{'ok': true, 'windowId': window.objectId};
      default:
        throw DebugActionException('unknown window action: $action');
    }
  }

  DenialWindow? _selectWindow(DebugActionContext context, int? windowId) {
    final shell = context.ref.read(shellControllerProvider);
    for (final window in shell.openAppWindows) {
      if (windowId != null) {
        if (window.objectId == windowId) {
          return window;
        }
      } else if (window.isUserApp) {
        return window;
      }
    }
    return null;
  }
}

Map<String, Object?> _rectJson(Rect rect) => <String, Object?>{
      'x': rect.left.round(),
      'y': rect.top.round(),
      'w': rect.width.round(),
      'h': rect.height.round(),
    };
