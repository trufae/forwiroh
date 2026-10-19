package org.forwiroh;

import android.content.Intent;
import android.Manifest;
import android.os.Build;
import androidx.test.rule.GrantPermissionRule;
import androidx.test.core.app.ActivityScenario;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.Rule;
import androidx.test.rule.ServiceTestRule;
import java.util.concurrent.TimeUnit;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class ActivityTest {
    @Rule public final GrantPermissionRule notifications = GrantPermissionRule.grant(
            Build.VERSION.SDK_INT >= 33 ? new String[]{Manifest.permission.POST_NOTIFICATIONS} : new String[0]);
    @Rule public final ServiceTestRule serviceRule = new ServiceTestRule();

    @Test public void notificationStopWorksWhileActivityRemainsBound() throws Exception {
        android.content.Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        try (ActivityScenario<MainActivity> activity = ActivityScenario.launch(MainActivity.class)) {
            NetworkService service = ((NetworkService.LocalBinder)serviceRule.bindService(
                    new Intent(context, NetworkService.class))).service();
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20);
            while (!service.ready() && !service.stopped() && System.nanoTime() < deadline) Thread.sleep(50);
            assertTrue("Core did not start: " + service.takeError(), service.ready());
            activity.onActivity(a -> a.startService(new Intent(a, NetworkService.class).setAction(NetworkService.STOP)));
            deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5);
            while (!service.stopped() && System.nanoTime() < deadline) Thread.sleep(25);
            assertTrue(service.stopped());
            assertFalse(service.ready());
        } finally { serviceRule.unbindService(); context.stopService(new Intent(context, NetworkService.class)); }
    }

    @Test public void opensAndSurvivesRecreation() {
        try (ActivityScenario<MainActivity> activity = ActivityScenario.launch(MainActivity.class)) {
            activity.onActivity(a -> assertNotNull(a.findViewById(android.R.id.content)));
            activity.recreate();
            activity.onActivity(a -> assertFalse(a.isFinishing()));
        } finally {
            InstrumentationRegistry.getInstrumentation().getTargetContext().stopService(
                    new Intent(InstrumentationRegistry.getInstrumentation().getTargetContext(), NetworkService.class));
        }
    }
}
