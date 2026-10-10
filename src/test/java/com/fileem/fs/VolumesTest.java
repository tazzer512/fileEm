package com.fileem.fs;

import com.fileem.util.OsUtil;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

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

    @Test
    void linuxEjectUnmountsEveryPartitionThenPowersOffTheDisk() {
        Path root = Path.of("/media/me/STICK");
        List<String> devs = List.of("/dev/sdb1", "/dev/sdb2");
        assertEquals(List.of(
                        List.of("udisksctl", "unmount", "--no-user-interaction", "-b", "/dev/sdb1"),
                        List.of("udisksctl", "unmount", "--no-user-interaction", "-b", "/dev/sdb2"),
                        List.of("udisksctl", "power-off", "--no-user-interaction", "-b", "/dev/sdb")),
                Volumes.linuxEjectCommands(root, devs, "/dev/sdb", Set.of("udisksctl", "gio", "umount")::contains));
        assertEquals(List.of(List.of("gio", "mount", "--eject", root.toString())),
                Volumes.linuxEjectCommands(root, devs, "/dev/sdb", Set.of("gio", "umount")::contains));
        assertEquals(List.of(List.of("gio", "mount", "--eject", root.toString())),
                Volumes.linuxEjectCommands(root, List.of(), null, Set.of("udisksctl", "gio")::contains), "no device: gio");
        assertEquals(List.of(List.of("umount", root.toString())),
                Volumes.linuxEjectCommands(root, devs, "/dev/sdb", Set.of("umount")::contains));
        assertTrue(Volumes.linuxEjectCommands(root, devs, "/dev/sdb", p -> false).isEmpty());
    }

    @Test
    void sameDriveFindsTheOtherMountedPartitionsOfTheDisk() {
        assumeTrue(!OsUtil.isWindows());
        List<String> mounts = List.of(
                "/dev/nvme0n1p2 / ext4 rw 0 0",
                "/dev/sdc1 /media/me/photos exfat rw 0 0",
                "/dev/sdd1 /media/me/other exfat rw 0 0",
                "/dev/sdc2 /media/me/storage exfat rw 0 0",
                "tmpfs /media/me/storage/cache tmpfs rw 0 0");
        java.util.function.Function<String, String> diskOf = d -> d.replaceAll("p?[0-9]+$", "");
        assertEquals(List.of(Path.of("/media/me/storage"), Path.of("/media/me/photos")),
                Volumes.sameDriveMounts(mounts, Path.of("/media/me/storage"), diskOf));
        assertEquals(List.of(Path.of("/media/me/other")), Volumes.sameDriveMounts(mounts, Path.of("/media/me/other"), diskOf));
        assertEquals(List.of(Path.of("/media/me/gone")), Volumes.sameDriveMounts(mounts, Path.of("/media/me/gone"), diskOf));
    }

    @Test
    void deviceForReadsEscapedMountPointsAndTakesTheTopMount() {
        assumeTrue(!OsUtil.isWindows());
        List<String> mounts = List.of(
                "/dev/nvme0n1p2 / ext4 rw 0 0",
                "/dev/sdb1 /media/me/MY\\040STICK vfat rw 0 0",
                "/dev/sdc1 /mnt/x ext4 rw 0 0",
                "/dev/sdd1 /mnt/x ext4 rw 0 0");
        assertEquals("/dev/sdb1", Volumes.deviceFor(mounts, Path.of("/media/me/MY STICK")));
        assertEquals("/dev/sdd1", Volumes.deviceFor(mounts, Path.of("/mnt/x")), "the last mount is the visible one");
        assertNull(Volumes.deviceFor(mounts, Path.of("/media/me/OTHER")));
    }

    @Test
    void ejectErrorsAreExplainedForPeople() {
        assertTrue(Volumes.ejectError("Error unmounting /dev/sdb1: target is busy").contains("in use"));
        assertTrue(Volumes.ejectError("Not authorized to perform operation").contains("not allowed"));
        assertEquals("something odd", Volumes.ejectError("  something odd\n"));
        assertFalse(Volumes.ejectError("").isBlank());
    }

    @Test
    void onlyRemovableVolumesAreEjectableByDefault() {
        assertTrue(new Volumes.Volume(Path.of("/media/x"), "x", Volumes.Kind.REMOVABLE, 1, 2).ejectable());
        assertFalse(new Volumes.Volume(Path.of("/"), "/", Volumes.Kind.SYSTEM, 1, 2).ejectable());
        assertFalse(new Volumes.Volume(Path.of("/net"), "n", Volumes.Kind.NETWORK, 1, 2).ejectable());
        for (Volumes.Volume v : Volumes.list()) {
            if (v.kind() == Volumes.Kind.SYSTEM) {
                assertFalse(v.ejectable(), "the system drive is never ejectable");
            }
        }
    }
}
