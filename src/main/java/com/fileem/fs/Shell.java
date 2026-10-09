package com.fileem.fs;

import com.fileem.util.Bg;
import com.fileem.util.OsUtil;
import javafx.application.Platform;

import java.awt.Desktop;
import java.io.IOException;
import java.nio.file.Path;
import java.util.function.Consumer;

/** Hands files to the operating system. */
public final class Shell {
    private Shell() {
    }

    /** Opens the file with its default application. Errors are reported on the FX thread. */
    public static void open(Path p, Consumer<String> onError) {
        Bg.POOL.execute(() -> {
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

    private static void exec(String... cmd) throws IOException {
        new ProcessBuilder(cmd)
                .redirectErrorStream(true)
                .redirectOutput(ProcessBuilder.Redirect.DISCARD)
                .start();
    }
}
