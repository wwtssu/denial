/*
 * denia-input: synthetic pointer via /dev/uinput (REL device).
 *
 * The device is created per invocation and destroyed on exit, so the
 * compositor (libinput) sees a normal evdev pointer appearing, moving,
 * clicking, and leaving. Events are delivered through the real input
 * pipeline — the same path as a physical mouse.
 *
 * Commands:
 *   denia-input move DX DY     relative pointer motion
 *   denia-input button 0|1     left button up/down
 *   denia-input click          left click
 *   denia-input drag DX DY     press, move in 8 steps, release
 *   denia-input abs X Y        absolute pointer motion (no accel, 1:1)
 *   denia-input adrag X Y      press, move absolutely to X,Y, release
 *   denia-input pdrag X1,Y1 X2,Y2 [X3,Y3 ...] [--ms N]
 *                              phased press→glide→release: abs to X1,Y1,
 *                              press, glide through each point in turn,
 *                              release. N ms (default 500) between phases
 *                              so an external harness can observe the
 *                              geometry after press / mid-drag / release.
 *   denia-input glide FX,FY TX,TY [--steps N] [--ms M]
 *                              progressive human-like motion: N abs steps
 *                              (default 16) interpolating from (FX,FY) to
 *                              (TX,TY), M ms (default 30) apart — visible
 *                              movement instead of a teleport.
 *
 * Absolute commands create an ABS device whose coordinates map 1:1 onto
 * the logical desktop (position_transformed), bypassing libinput pointer
 * acceleration entirely — useful for reaching exact points (e.g. the
 * top resize band inside the shell title bar) without feedback loops.
 *
 * Build: cc denia-input.c -o denia-input
 */
#include <fcntl.h>
#include <linux/input.h>
#include <linux/uinput.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

static int fd = -1;
static int use_abs = 0;

static void die(const char *message) {
  perror(message);
  exit(1);
}

static void emit(int type, int code, int value) {
  struct input_event ev;
  memset(&ev, 0, sizeof(ev));
  ev.type = type;
  ev.code = code;
  ev.value = value;
  if (write(fd, &ev, sizeof(ev)) != (ssize_t)sizeof(ev)) {
    die("write event");
  }
}

static void sync_events(void) { emit(EV_SYN, SYN_REPORT, 0); }

static void open_device(void) {
  fd = open("/dev/uinput", O_WRONLY | O_NONBLOCK);
  if (fd < 0) {
    die("open /dev/uinput");
  }
  ioctl(fd, UI_SET_EVBIT, EV_KEY);
  ioctl(fd, UI_SET_KEYBIT, BTN_LEFT);
  ioctl(fd, UI_SET_KEYBIT, BTN_RIGHT);
  if (use_abs) {
    ioctl(fd, UI_SET_EVBIT, EV_ABS);
    ioctl(fd, UI_SET_ABSBIT, ABS_X);
    ioctl(fd, UI_SET_ABSBIT, ABS_Y);
    struct uinput_abs_setup absx;
    memset(&absx, 0, sizeof(absx));
    absx.code = ABS_X;
    absx.absinfo.minimum = 0;
    absx.absinfo.maximum = 1920;
    absx.absinfo.fuzz = 0;
    absx.absinfo.flat = 0;
    if (ioctl(fd, UI_ABS_SETUP, &absx) < 0) {
      die("UI_ABS_SETUP ABS_X");
    }
    absx.code = ABS_Y;
    absx.absinfo.maximum = 1280;
    if (ioctl(fd, UI_ABS_SETUP, &absx) < 0) {
      die("UI_ABS_SETUP ABS_Y");
    }
  } else {
    ioctl(fd, UI_SET_EVBIT, EV_REL);
    ioctl(fd, UI_SET_RELBIT, REL_X);
    ioctl(fd, UI_SET_RELBIT, REL_Y);
  }

  struct uinput_setup setup;
  memset(&setup, 0, sizeof(setup));
  setup.id.bustype = BUS_USB;
  setup.id.vendor = 0xde1a;
  setup.id.product = use_abs ? 0x0002 : 0x0001;
  setup.id.version = 1;
  snprintf(setup.name, UINPUT_MAX_NAME_SIZE, use_abs ? "denia-input abs mouse" : "denia-input virtual mouse");
  if (ioctl(fd, UI_DEV_SETUP, &setup) < 0) {
    die("UI_DEV_SETUP");
  }
  if (ioctl(fd, UI_DEV_CREATE) < 0) {
    die("UI_DEV_CREATE");
  }
  /* Give libinput a moment to recognize the new device. */
  usleep(250000);
}

static void close_device(void) {
  if (fd < 0) {
    return;
  }
  ioctl(fd, UI_DEV_DESTROY);
  close(fd);
  fd = -1;
}

static void move_relative(int dx, int dy) {
  if (dx != 0) {
    emit(EV_REL, REL_X, dx);
  }
  if (dy != 0) {
    emit(EV_REL, REL_Y, dy);
  }
  sync_events();
}

static void move_absolute(int x, int y) {
  emit(EV_ABS, ABS_X, x);
  emit(EV_ABS, ABS_Y, y);
  sync_events();
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr,
            "usage: denia-input move DX DY | button 0|1 | click | drag DX DY\n");
    return 2;
  }

  const char *cmd = argv[1];
  use_abs = (strcmp(cmd, "abs") == 0) || (strcmp(cmd, "adrag") == 0) ||
            (strcmp(cmd, "pdrag") == 0) || (strcmp(cmd, "glide") == 0);
  open_device();

  if (strcmp(cmd, "move") == 0 && argc == 4) {
    move_relative(atoi(argv[2]), atoi(argv[3]));
  } else if (strcmp(cmd, "abs") == 0 && argc == 4) {
    move_absolute(atoi(argv[2]), atoi(argv[3]));
  } else if (strcmp(cmd, "button") == 0 && argc == 3) {
    emit(EV_KEY, BTN_LEFT, atoi(argv[2]) ? 1 : 0);
    sync_events();
  } else if (strcmp(cmd, "click") == 0) {
    emit(EV_KEY, BTN_LEFT, 1);
    sync_events();
    usleep(60000);
    emit(EV_KEY, BTN_LEFT, 0);
    sync_events();
  } else if (strcmp(cmd, "drag") == 0 && argc == 4) {
    const int dx = atoi(argv[2]);
    const int dy = atoi(argv[3]);
    const int steps = 8;
    emit(EV_KEY, BTN_LEFT, 1);
    sync_events();
    usleep(80000);
    for (int step = 1; step <= steps; step++) {
      move_relative(dx / steps, dy / steps);
      usleep(20000);
    }
    move_relative(dx - (dx / steps) * steps, dy - (dy / steps) * steps);
    usleep(60000);
    emit(EV_KEY, BTN_LEFT, 0);
    sync_events();
  } else if (strcmp(cmd, "adrag") == 0 && argc == 4) {
    const int tx = atoi(argv[2]);
    const int ty = atoi(argv[3]);
    /* 1:1 absolute drag: press, glide in 4 steps, release. */
    int x0 = -1, y0 = -1;
    const char *from = getenv("DENIA_INPUT_FROM");
    if (from != NULL && sscanf(from, "%d,%d", &x0, &y0) == 2) {
      move_absolute(x0, y0);
      usleep(40000);
    }
    emit(EV_KEY, BTN_LEFT, 1);
    sync_events();
    usleep(80000);
    if (x0 >= 0) {
      for (int step = 1; step <= 4; step++) {
        move_absolute(x0 + (tx - x0) * step / 4, y0 + (ty - y0) * step / 4);
        usleep(20000);
      }
    } else {
      move_absolute(tx, ty);
      usleep(40000);
    }
    emit(EV_KEY, BTN_LEFT, 0);
    sync_events();
  } else if (strcmp(cmd, "pdrag") == 0 && argc >= 4) {
    /* pdrag X1,Y1 X2,Y2 [X3,Y3 ...] [--ms N] */
    int ms = 500;
    int npts = 0;
    int pts[16][2];
    for (int i = 2; i < argc && npts < 16; i++) {
      if (strcmp(argv[i], "--ms") == 0 && i + 1 < argc) {
        ms = atoi(argv[++i]);
        continue;
      }
      if (sscanf(argv[i], "%d,%d", &pts[npts][0], &pts[npts][1]) != 2) {
        fprintf(stderr, "bad point: %s\n", argv[i]);
        close_device();
        return 2;
      }
      npts++;
    }
    if (npts < 2) {
      fprintf(stderr, "pdrag needs at least two points\n");
      close_device();
      return 2;
    }
    move_absolute(pts[0][0], pts[0][1]);
    usleep((useconds_t)ms * 1000);
    emit(EV_KEY, BTN_LEFT, 1);
    sync_events();
    usleep((useconds_t)ms * 1000);
    for (int i = 1; i < npts; i++) {
      move_absolute(pts[i][0], pts[i][1]);
      usleep((useconds_t)ms * 1000);
    }
    emit(EV_KEY, BTN_LEFT, 0);
    sync_events();
    usleep((useconds_t)ms * 1000);
  } else if (strcmp(cmd, "glide") == 0 && argc >= 5) {
    /* glide FX,FY TX,TY [--steps N] [--ms M] */
    int fx = -1, fy = -1, tx = 0, ty = 0;
    int steps = 16, ms = 30;
    for (int i = 2; i < argc; i++) {
      if (strcmp(argv[i], "--steps") == 0 && i + 1 < argc) {
        steps = atoi(argv[++i]);
        continue;
      }
      if (strcmp(argv[i], "--ms") == 0 && i + 1 < argc) {
        ms = atoi(argv[++i]);
        continue;
      }
      if (fx < 0) {
        if (sscanf(argv[i], "%d,%d", &fx, &fy) != 2) {
          fprintf(stderr, "bad point: %s\n", argv[i]);
          close_device();
          return 2;
        }
      } else if (sscanf(argv[i], "%d,%d", &tx, &ty) != 2) {
        fprintf(stderr, "bad point: %s\n", argv[i]);
        close_device();
        return 2;
      }
    }
    if (fx < 0 || steps < 1 || ms < 1) {
      fprintf(stderr, "glide needs FX,FY TX,TY\n");
      close_device();
      return 2;
    }
    for (int s = 1; s <= steps; s++) {
      move_absolute(fx + (tx - fx) * s / steps, fy + (ty - fy) * s / steps);
      usleep((useconds_t)ms * 1000);
    }
  } else {
    fprintf(stderr,
            "usage: denia-input move DX DY | abs X Y | button 0|1 | click | drag DX DY | adrag X Y | pdrag X1,Y1 X2,Y2 [..] [--ms N] | glide FX,FY TX,TY [--steps N] [--ms M]\n");
    close_device();
    return 2;
  }

  /* Keep the device alive long enough for the compositor to dispatch the
   * events. A too-short lifetime races the libinput polling cycle and the
   * events get dropped with the device (observed with the default 120ms).
   * 800ms comfortably covers one dispatch round-trip. */
  usleep(800000);
  const char *hold = getenv("DENIA_INPUT_HOLD_MS");
  if (hold != NULL) {
    usleep((useconds_t)atoi(hold) * 1000);
  }
  close_device();
  return 0;
}
