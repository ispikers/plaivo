package xyz.spikers.plaivo

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.Listing
import uniffi.plaivo_ffi.discoverId
import uniffi.plaivo_ffi.librarySorts

private val KINDS = listOf("Movies", "TV Shows")

@Composable
fun WatchlistScreen(open: (MediaCard) -> Unit) {
    val discover = remember { discoverId() }
    val sorts = remember { librarySorts() }
    val labels = remember(sorts) { sorts.map { it.label } }
    var sort by rememberSaveable { mutableStateOf("addedAt:desc") }
    var shows by rememberSaveable { mutableStateOf(false) }
    val listing = remember(sort, shows) { Listing.Watchlist(sort, shows) }
    Column(Modifier.fillMaxSize()) {
        TabHeader("Watchlist", KINDS, if (shows) 1 else 0) { shows = it == 1 }
        Box(Modifier.weight(1f)) {
            PagedGrid(discover, "", listing, open) { status ->
                SortRow(status, labels, sorts.indexOfFirst { it.id == sort }) { sort = sorts[it].id }
            }
        }
    }
}
