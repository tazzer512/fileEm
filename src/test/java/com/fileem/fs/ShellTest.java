package com.fileem.fs;

import com.fileem.util.OsUtil;
import org.junit.jupiter.api.Test;

import java.nio.file.Path;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

class ShellTest {
    private static final Path DIR = Path.of("/work/dir");

    @Test
    void showInFileManagerQuotesApostrophesInTheItemUri() {
        assumeTrue(OsUtil.isPosix());
        String arg = Shell.showItemsArgument(Path.of("/home/u/it's here.txt"));
        assertEquals("['file:///home/u/it%27s%20here.txt']", arg);
    }

    @Test
    void linuxPrefersTheTerminalEnvironmentVariable() {
        assumeTrue(OsUtil.isLinux());
        assertEquals(List.of("foot"), Shell.terminalCommand(DIR, "foot", Set.of("foot", "xterm")::contains));
    }

    @Test
    void linuxPassesTheFolderToKnownTerminals() {
        assumeTrue(OsUtil.isLinux());
        assertEquals(List.of("konsole", "--workdir", DIR.toString()),
                Shell.terminalCommand(DIR, null, Set.of("konsole", "xterm")::contains));
        assertEquals(List.of("gnome-terminal", "--working-directory=" + DIR),
                Shell.terminalCommand(DIR, "not-installed", Set.of("gnome-terminal")::contains));
        assertNull(Shell.terminalCommand(DIR, null, p -> false));
    }

    @Test
    void semicolonsInAFolderForWindowsTerminalAreEscaped() {
        assertEquals("C:\\a\\;calc", Shell.windowsTerminalEscape("C:\\a;calc"));
        assertEquals("C:\\Users\\me", Shell.windowsTerminalEscape("C:\\Users\\me"));
    }

    @Test
    void macAndWindowsAlwaysHaveATerminal() {
        assumeTrue(!OsUtil.isLinux());
        assertEquals(OsUtil.isMac() ? "open" : "cmd.exe",
                Shell.terminalCommand(DIR, null, p -> false).get(0));
    }
}
