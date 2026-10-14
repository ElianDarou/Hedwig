package de.hedwig.app

import android.app.Application
import androidx.work.Configuration

class HedwigApplication : Application(), Configuration.Provider {
    override val workManagerConfiguration: Configuration
        get() = Configuration.Builder().setMinimumLoggingLevel(android.util.Log.INFO).build()
}
