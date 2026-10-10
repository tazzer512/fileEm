package com.fileem.fs;

import com.fileem.util.Bg;
import com.fileem.util.OsUtil;
import javafx.application.Platform;

import java.awt.Desktop;
import java.io.IOException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;
import java.util.function.Predicate;

/** Hands files to the operating system. */
public final class Shell {
    /**
     * Set by the test runner ({@code -Dfileem.noLaunch=true}): nothing is started, requests are only
     * recorded in {@link #launched()}. Tests must never open files in the user's real applications.
     */
    private static final boolean NO_LAUNCH = Boolean.getBoolean("fileem.noLaunch");
    private static final List<String> LAUNCHED = new CopyOnWriteArrayList<>();

    private Shell() {
    }

    /** What was requested while launching is disabled ("open /path", "terminal /path", "reveal /path"). */
    public static List<String> launched() {
        return List.copyOf(LAUNCHED);
    }

    public static void clearLaunched() {
        LAUNCHED.clear();
    }

    private static boolean suppressed(String what, Path p) {
        if (NO_LAUNCH) {
            LAUNCHED.add(what + " " + p);
        }
        return NO_LAUNCH;
    }

    /** Opens the file with its default application. Errors are reported on the FX thread. */
    public static void open(Path p, Consumer<String> onError) {
        if (suppressed("open", p)) {
            return;
        }
        Bg.IO.execute(() -> {
            try {
                if (OsUtil.isLinux()) {
                    try {
                        exec("xdg-open", p.toString());
                        return;
                    } catch (IOException e) {
                        // no xdg-open, try AWT below
                    }
                } else if (OsUtil.isMac()) {
                    exec("open", p.toString());
                    return;
                }
                Desktop.getDesktop().open(p.toFile());
            } catch (Exception e) {
                String msg = e.getMessage() != null ? e.getMessage() : e.toString();
                Platform.runLater(() -> onError.accept(msg));
            }
        });
    }

    /** Label for "show this item in the OS file manager", per platform. */
    public static String fileManagerName() {
        return OsUtil.isWindows() ? "Explorer" : OsUtil.isMac() ? "Finder" : "file manager";
    }

    /** Opens a terminal window in {@code dir}. */
    public static void openTerminal(Path dir, Consumer<String> onError) {
        if (suppressed("terminal", dir)) {
            return;
        }
        Bg.IO.execute(() -> {
            try {
                List<String> cmd = terminalCommand(dir, System.getenv("TERMINAL"), OsUtil::onPath);
                if (cmd == null) {
                    throw new IOException("No terminal application was found. Set the TERMINAL environment variable.");
                }
                new ProcessBuilder(cmd).directory(dir.toFile())
                        .redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
            } catch (Exception e) {
                report(e, onError);
            }
        });
    }

    /**
     * The command that opens a terminal in {@code dir}, or null when none is installed.
     * The process is also started with {@code dir} as working directory, for terminals without a flag.
     */
    static List<String> terminalCommand(Path dir, String envTerminal, Predicate<String> installed) {
        String d = dir.toString();
        if (OsUtil.isWindows()) {
            return installed.test("wt.exe") ? List.of("wt.exe", "-d", d) : List.of("cmd.exe", "/c", "start", "cmd.exe");
        }
        if (OsUtil.isMac()) {
            return List.of("open", "-a", "Terminal", d);
        }
        if (envTerminal != null && !envTerminal.isBlank() && installed.test(envTerminal)) {
            return List.of(envTerminal);
        }
        Map<String, List<String>> known = new LinkedHashMap<>();
        known.put("x-terminal-emulator", List.of());
        known.put("gnome-terminal", List.of("--working-directory=" + d));
        known.put("konsole", List.of("--workdir", d));
        known.put("xfce4-terminal", List.of("--working-directory=" + d));
        known.put("tilix", List.of("-w", d));
        known.put("kitty", List.of("--directory", d));
        known.put("alacritty", List.of("--working-directory", d));
        known.put("wezterm", List.of("start", "--cwd", d));
        known.put("foot", List.of("--working-directory=" + d));
        known.put("xterm", List.of());
        for (Map.Entry<String, List<String>> e : known.entrySet()) {
            if (installed.test(e.getKey())) {
                List<String> cmd = new ArrayList<>();
                cmd.add(e.getKey());
                cmd.addAll(e.getValue());
                return cmd;
            }
        }
        return null;
    }

    /** Shows the item selected in the OS file manager (falls back to opening its folder). */
    public static void reveal(Path p, Consumer<String> onError) {
        if (suppressed("reveal", p)) {
            return;
        }
        Bg.IO.execute(() -> {
            try {
                if (OsUtil.isWindows()) {
                    exec("explorer.exe", "/select," + p);
                } else if (OsUtil.isMac()) {
                    exec("open", "-R", p.toString());
                } else if (!freedesktopShowItem(p)) {
                    Path folder = p.getParent() != null ? p.getParent() : p;
                    exec("xdg-open", folder.toString());
                }
            } catch (Exception e) {
                report(e, onError);
            }
        });
    }

    private static boolean freedesktopShowItem(Path p) {
        try {
            Process proc = new ProcessBuilder("gdbus", "call", "--session",
                    "--dest", "org.freedesktop.FileManager1",
                    "--object-path", "/org/freedesktop/FileManager1",
                    "--method", "org.freedesktop.FileManager1.ShowItems",
                    showItemsArgument(p), "")
                    .redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.DISCARD).start();
            return proc.waitFor(5, TimeUnit.SECONDS) && proc.exitValue() == 0;
        } catch (IOException e) {
            return false;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return false;
        }
    }

    /**
     * The GVariant array literal for ShowItems. URIs keep {@code '} unescaped, which would end the
     * quoted string early (the call fails or names other items), so it is percent-encoded too.
     */
    static String showItemsArgument(Path p) {
        return "['" + p.toUri().toString().replace("'", "%27") + "']";
    }

    private static void report(Exception e, Consumer<String> onError) {
        String msg = e.getMessage() != null ? e.getMessage() : e.toString();
        Platform.runLater(() -> onError.accept(msg));
    }

    private static void exec(String... cmd) throws IOException {
        new ProcessBuilder(cmd)
                .redirectErrorStream(true)
                .redirectOutput(ProcessBuilder.Redirect.DISCARD)
                .start();
    }
}
