package com.fileem.ui;

import com.fileem.FxTestSupport;
import javafx.scene.control.Alert;
import javafx.scene.image.Image;
import javafx.stage.Stage;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import org.testfx.util.WaitForAsyncUtils;

import java.util.List;
import java.util.concurrent.TimeUnit;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;

class IconsTest {
    @BeforeAll
    static void startFx() {
        FxTestSupport.startToolkit();
    }

    @Test
    void appIconLoadsInEverySize() {
        List<Image> icons = Icons.appIcons();
        assertEquals(8, icons.size(), "every size is bundled");
        int[] sizes = {16, 24, 32, 48, 64, 128, 256, 512};
        for (int i = 0; i < sizes.length; i++) {
            Image img = icons.get(i);
            assertFalse(img.isError(), "size " + sizes[i]);
            assertEquals(sizes[i], img.getWidth(), 0);
            assertEquals(sizes[i], img.getHeight(), 0);
        }
        assertSame(icons, Icons.appIcons(), "loaded once");
    }

    @Test
    void dialogsGetTheAppIcon() throws Exception {
        List<Image> icons = WaitForAsyncUtils.asyncFx(() -> {
            Alert a = new Alert(Alert.AlertType.INFORMATION);
            Dialogs.prepare(a, null);
            return List.copyOf(((Stage) a.getDialogPane().getScene().getWindow()).getIcons());
        }).get(10, TimeUnit.SECONDS);
        assertEquals(Icons.appIcons(), icons);
    }
}
