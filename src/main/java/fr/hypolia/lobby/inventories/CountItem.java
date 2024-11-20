package fr.hypolia.lobby.inventories;

import org.bukkit.Material;
import org.bukkit.entity.Player;
import org.bukkit.event.inventory.ClickType;
import org.bukkit.event.inventory.InventoryClickEvent;
import org.jetbrains.annotations.NotNull;
import xyz.xenondevs.invui.item.ItemProvider;
import xyz.xenondevs.invui.item.builder.ItemBuilder;
import xyz.xenondevs.invui.item.impl.AbstractItem;

public class CountItem extends AbstractItem {
  private int count;

  @Override
  public ItemProvider getItemProvider() {
    return new ItemBuilder(Material.DIAMOND).setDisplayName("§bCount: §f" + count);
  }

  @Override
  public void handleClick(@NotNull ClickType clickType, @NotNull Player player, @NotNull InventoryClickEvent event) {
    if (clickType.isLeftClick()) {
      count++; // increment if left click
    } else {
      count--; // else decrement
    }

    notifyWindows(); // this will update the ItemStack that is displayed to the player
  }
}
