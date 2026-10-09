package com.fileem.fs;

import com.fileem.util.OsUtil;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class VolumesTest {
    @Test
    void listsAtLeastTheSystemVolumeWithUniqueExistingRoots() {
        List<Volumes.Volume> vols = Volumes.list();
        assertFalse(vols.isEmpty());
        assertEquals(vols.size(), new HashSet<>(vols.stream().map(Volumes.Volume::root).toList()).size(),
                "no duplicate mount points");
        for (Volumes.Volume v : vols) {
            assertFalse(v.label().isBlank(), "label for " + v.root());
            assertTrue(v.root().isAbsolute());
        }
        if (!OsUtil.isWindows()) {
            assertEquals(Path.of("/"), vols.get(0).root(), "the root file system comes first");
            assertTrue(Files.isDirectory(vols.get(0).root()));
            assertTrue(vols.get(0).total() > 0, "space is reported for the root");
        }
    }
}
