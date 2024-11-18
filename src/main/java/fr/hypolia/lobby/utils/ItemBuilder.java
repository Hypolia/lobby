package fr.hypolia.lobby.utils;

import org.bukkit.Material;
import org.bukkit.OfflinePlayer;
import org.bukkit.enchantments.Enchantment;
import org.bukkit.inventory.ItemFlag;
import org.bukkit.inventory.ItemStack;
import org.bukkit.inventory.meta.ItemMeta;
import org.bukkit.inventory.meta.SkullMeta;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public class ItemBuilder {
  private final ItemStack itemStack;
  private ItemMeta itemMeta;

  /**
   * @param material The material of the item
   */
  public ItemBuilder (Material material) {
    this.itemStack = new ItemStack(material);
    this.itemMeta = this.itemStack.getItemMeta();
  }

  /**
   * Définir la quantité de l'objet.
   *
   * @param amount Quantité à définir.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder setAmount (int amount) {
    this.itemStack.setAmount(amount);
    return this;
  }

  /**
   * Définir le nom de l'objet.
   *
   * @param name Nom de l'objet.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder setName (String name) {
    this.itemMeta.setDisplayName(name);
    return this;
  }

  /**
   * Ajouter une description (lore) à l'objet.
   *
   * @param lore Liste de chaînes représentant la description.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder setLore (List<String> lore) {
    this.itemMeta.setLore(lore);
    return this;
  }

  /**
   * Ajouter une description (lore) simple à l'objet.
   *
   * @param loreArray Tableau de chaînes représentant la description.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder setLore (String... loreArray) {
    List<String> lore = new ArrayList<>();
    Collections.addAll(lore, loreArray);
    this.itemMeta.setLore(lore);
    return this;
  }

  /**
   * Ajouter un enchantement visible ou non visible.
   *
   * @param enchant            Enchantement à ajouter.
   * @param level              Niveau de l'enchantement.
   * @param ignoreRestrictions Ignorer les restrictions de niveau d'enchantement.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder addEnchantment (Enchantment enchant, int level, boolean ignoreRestrictions) {
    this.itemMeta.addEnchant(enchant, level, ignoreRestrictions);
    return this;
  }

  /**
   * Ajouter un drapeau pour masquer les informations sur l'objet.
   *
   * @param flags Les drapeaux à ajouter.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder addItemFlags (ItemFlag... flags) {
    this.itemMeta.addItemFlags(flags);
    return this;
  }

  /**
   * Rendre l'objet incassable.
   *
   * @param unbreakable Si l'objet est incassable.
   * @return L'instance actuelle de ItemBuilder.
   */
  public ItemBuilder setUnbreakable (boolean unbreakable) {
    this.itemMeta.setUnbreakable(unbreakable);
    return this;
  }

  public ItemBuilder setPlayerHead(OfflinePlayer player) {
    if (this.itemStack.getType() != Material.PLAYER_HEAD) {
      throw new IllegalStateException("setPlayerHead() can only be used with a PLAYER_HEAD item.");
    }

    SkullMeta skullMeta = (SkullMeta) this.itemMeta;
    skullMeta.setOwningPlayer(player);
    this.itemMeta = skullMeta;
    return this;
  }

  /**
   * Construire l'ItemStack final avec toutes les modifications.
   *
   * @return L'ItemStack configuré.
   */
  public ItemStack build () {
    this.itemStack.setItemMeta(this.itemMeta);
    return this.itemStack;
  }
}
