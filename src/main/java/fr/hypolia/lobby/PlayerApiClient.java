package fr.hypolia.lobby;

import fr.hypolia.lobby.services.KeycloakService;
import org.apache.hc.client5.http.fluent.Request;
import org.apache.hc.client5.http.fluent.Response;
import org.apache.http.client.HttpResponseException;

import java.io.IOException;
import java.util.UUID;

public class PlayerApiClient {
  private final String apiBaseUrl;
  private final KeycloakService keycloakService;

  public PlayerApiClient(String apiBaseUrl, KeycloakService keycloakService) {
    this.apiBaseUrl = apiBaseUrl;
    this.keycloakService = keycloakService;
  }

  public void getPlayerInfo(UUID uuid) throws Exception {
    String url = apiBaseUrl + "/v1/players/" + uuid.toString();

    String accessToken = keycloakService.getAccessToken();

    try {
      // Call API
      Response response = Request.get(url)
          .addHeader("Authorization", "Bearer " + accessToken)
          .addHeader("Content-Type", "application/json")
          .execute();

      // print response body
      System.out.println(response.returnContent().asString());

    } catch (HttpResponseException e) {
      throw new IOException("Erreur API: " + e.getMessage(), e);
    }
  }
}
