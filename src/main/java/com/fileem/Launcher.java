package com.fileem;

/**
 * Plain entry point so the app can start from a classpath jar without the
 * JavaFX "runtime components are missing" check on the Application subclass.
 */
public final class Launcher {
    private Launcher() {
    }

    public static void main(String[] args) {
        FileEmApp.main(args);
    }
}
