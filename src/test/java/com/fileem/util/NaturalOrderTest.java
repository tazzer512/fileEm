package com.fileem.util;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

class NaturalOrderTest {
    private static final NaturalOrder N = NaturalOrder.INSTANCE;

    @Test
    void comparesDigitRunsNumerically() {
        List<String> names = new ArrayList<>(List.of("file10.txt", "file2.txt", "file1.txt", "file02.txt", "file100"));
        names.sort(N);
        // equal numbers with different leading zeros are ordered deterministically by their text
        assertEquals(List.of("file1.txt", "file02.txt", "file2.txt", "file10.txt", "file100"), names);
    }

    @Test
    void ignoresCase() {
        assertTrue(N.compare("apple", "Banana") < 0);
        assertTrue(N.compare("Apple", "banana") < 0);
        assertTrue(N.compare("ABC", "abd") < 0);
    }

    @Test
    void shorterPrefixSortsFirstAndEqualIsZero() {
        assertTrue(N.compare("abc", "abcd") < 0);
        assertEquals(0, N.compare("same", "same"));
        assertTrue(N.compare("", "a") < 0);
    }

    @Test
    void handlesHugeNumbersWithoutOverflow() {
        assertTrue(N.compare("x99999999999999999999", "x100000000000000000000") < 0);
    }

    @Test
    void isAConsistentComparator() {
        Random r = new Random(42);
        String alphabet = "aAbB019 ._-";
        List<String> samples = new ArrayList<>();
        for (int i = 0; i < 300; i++) {
            StringBuilder sb = new StringBuilder();
            for (int j = r.nextInt(8); j > 0; j--) {
                sb.append(alphabet.charAt(r.nextInt(alphabet.length())));
            }
            samples.add(sb.toString());
        }
        for (String a : samples) {
            for (String b : samples) {
                assertEquals(Integer.signum(N.compare(a, b)), -Integer.signum(N.compare(b, a)), a + " vs " + b);
                if (N.compare(a, b) == 0) {
                    assertEquals(a, b, "only identical strings compare equal");
                }
            }
        }
        samples.sort(N); // TimSort throws when the contract is violated
    }
}
