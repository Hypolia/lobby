package fr.hypolia.lobby;

import fr.hypolia.lobby.utils.ItemBuilder;
import org.bukkit.Material;
import org.bukkit.entity.Player;
import org.bukkit.inventory.ItemFlag;
import org.bukkit.inventory.ItemStack;
import org.bukkit.inventory.meta.ItemMeta;

public class LobbyItems {
  public static void giveLobbyItems(Player player) {
    player.getInventory().clear();

    ItemStack compass = new ItemBuilder(Material.COMPASS)
        .setName("§bSélecteur de Mini-Jeux")
        .setLore("§7Utilisez cette boussole", "§7pour choisir un mini-jeu.")
        .setUnbreakable(true)
        .build();
    player.getInventory().setItem(0, compass);

    ItemStack playerHead = new ItemBuilder(Material.PLAYER_HEAD)
        .setName("§aVotre Tête")
        .setUnbreakable(true)
        .setPlayerHead(player)
        .build();
    player.getInventory().setItem(1, playerHead);

    // Slot 9: Étoile du Nether
    ItemStack netherStar = new ItemBuilder(Material.NETHER_STAR)
        .setName("§6Menu Principal")
        .addEnchantment(org.bukkit.enchantments.Enchantment.LUCK, 1, true)
        .addItemFlags(ItemFlag.HIDE_ENCHANTS) // Masquer l'enchantement
        .build();
    player.getInventory().setItem(8, netherStar); // Dernier slot de la barre rapide
  }
}
