package fr.hypolia.lobby.services;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import org.apache.hc.client5.http.entity.UrlEncodedFormEntity;
import org.apache.hc.client5.http.fluent.Request;
import org.apache.hc.core5.http.message.BasicNameValuePair;

import java.util.Arrays;

import static org.bukkit.Bukkit.getLogger;

public class KeycloakService {
  private final String serverUrl;
  private final String realm;
  private final String clientId;
  private final String clientSecret;

  private String accessToken;
  private String refreshToken;
  private long tokenExpiryTime;

  public KeycloakService(String serverUrl, String realm, String clientId, String clientSecret) {
    this.serverUrl = serverUrl;
    this.realm = realm;
    this.clientId = clientId;
    this.clientSecret = clientSecret;
  }

  public String getAccessToken() throws Exception {
    if (accessToken == null || isTokenExpired()) {
      refreshToken();
    }
    return accessToken;
  }


  /**
   * Checks if the current token is expired.
   *
   * @return true if the token is expired, false otherwise
   */
  private boolean isTokenExpired() {
    return System.currentTimeMillis() >= tokenExpiryTime;
  }

  /**
   * Fetch a new token using client credentials.
   *
   * @throws Exception if the token request fails
   */
  private void refreshToken() throws Exception {
    try {
      String response = Request.post(serverUrl + "/realms/" + realm + "/protocol/openid-connect/token")
          .body(new UrlEncodedFormEntity(Arrays.asList(
              new BasicNameValuePair("grant_type", refreshToken == null ? "client_credentials" : "refresh_token"),
              new BasicNameValuePair("client_id", clientId),
              new BasicNameValuePair("client_secret", clientSecret),
              refreshToken != null ? new BasicNameValuePair("refresh_token", refreshToken) : null
          )))
          .execute()
          .returnContent()
          .asString();

      parseTokenResponse(response);

    } catch (Exception e) {
      getLogger().severe("Failed to refresh access token from Keycloak: " + e.getMessage());
      throw e;
    }
  }

  /**
   * Parse the response from Keycloak and extract token information.
   *
   * @param response the JSON response string
   */
  private void parseTokenResponse(String response) {
    JsonObject jsonObject = JsonParser.parseString(response).getAsJsonObject();

    this.accessToken = jsonObject.get("access_token").getAsString();
    this.refreshToken = jsonObject.has("refresh_token") ? jsonObject.get("refresh_token").getAsString() : null;
    int expiresIn = jsonObject.get("expires_in").getAsInt();

    // Calculate token expiry time (current time + expires_in * 1000ms)
    this.tokenExpiryTime = System.currentTimeMillis() + (expiresIn * 1000L);

    getLogger().info("Access token refreshed successfully. Token expires in " + expiresIn + " seconds.");
  }
}
