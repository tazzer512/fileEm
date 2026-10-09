package com.fileem.util;

import java.util.Comparator;

/** Case-insensitive ordering that compares digit runs numerically, so "file2" sorts before "file10". */
public final class NaturalOrder implements Comparator<String> {
    public static final NaturalOrder INSTANCE = new NaturalOrder();

    private NaturalOrder() {
    }

    @Override
    public int compare(String a, String b) {
        int i = 0, j = 0;
        int na = a.length(), nb = b.length();
        while (i < na && j < nb) {
            char ca = a.charAt(i), cb = b.charAt(j);
            if (isDigit(ca) && isDigit(cb)) {
                int si = i, sj = j;
                while (si < na && a.charAt(si) == '0') si++;
                while (sj < nb && b.charAt(sj) == '0') sj++;
                int ei = si, ej = sj;
                while (ei < na && isDigit(a.charAt(ei))) ei++;
                while (ej < nb && isDigit(b.charAt(ej))) ej++;
                int la = ei - si, lb = ej - sj;
                if (la != lb) {
                    return la - lb;
                }
                for (int k = 0; k < la; k++) {
                    int d = a.charAt(si + k) - b.charAt(sj + k);
                    if (d != 0) {
                        return d;
                    }
                }
                i = ei;
                j = ej;
            } else {
                int d = Character.compare(fold(ca), fold(cb));
                if (d != 0) {
                    return d;
                }
                i++;
                j++;
            }
        }
        int rest = (na - i) - (nb - j);
        return rest != 0 ? rest : a.compareTo(b);
    }

    private static boolean isDigit(char c) {
        return c >= '0' && c <= '9';
    }

    private static char fold(char c) {
        return Character.toLowerCase(Character.toUpperCase(c));
    }
}
